//! wgpu Renderer module
//!
//! Handles GPU initialization and 3D rendering for the VR environment.
//! Supports both normal and stereoscopic VR rendering modes.

use std::sync::Arc;
use wgpu::{
    Backends, Device, DeviceDescriptor, Instance, InstanceDescriptor, Queue,
    RenderPipeline, Surface, SurfaceConfiguration, SurfaceTargetUnsafe, TextureUsages,
    BindGroup, BindGroupLayout, Buffer,
};
use winit::window::Window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use glam::{Mat4, Quat, Vec3};
use bytemuck::{Pod, Zeroable};

/// Camera uniform data sent to GPU
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
    eye_offset: [f32; 4], // x = eye offset, y = is_vr_mode
}

pub struct Renderer {
    #[allow(dead_code)]
    window: Arc<Window>,
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    pipeline: RenderPipeline,
    size: (u32, u32),
    
    // Camera uniforms
    camera_buffer: Buffer,
    camera_bind_group: BindGroup,
    
    // VR mode state
    pub vr_mode: bool,
}

impl Renderer {
    // Inter-pupillary distance (average human IPD is ~63mm)
    const IPD: f32 = 0.063;
    
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        
        // Create wgpu instance (Vulkan only for Android)
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::VULKAN,
            ..Default::default()
        });
        
        // Create surface using raw handles
        let surface = unsafe {
            let window_handle = window.window_handle().unwrap().as_raw();
            let display_handle = window.display_handle().unwrap().as_raw();
            let target = SurfaceTargetUnsafe::RawHandle { 
                raw_display_handle: display_handle, 
                raw_window_handle: window_handle,
            };
            instance.create_surface_unsafe(target).unwrap()
        };
        
        // Get adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find GPU adapter");
        
        // Create device and queue
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default(), None)
            .await
            .expect("Failed to create device");
        
        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats[0];
        
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        
        // Create camera uniform buffer
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Buffer"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        
        // Create bind group
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        
        // Create render pipeline
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/main.wgsl").into()),
        });
        
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        
        Self {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            size: (size.width, size.height),
            camera_buffer,
            camera_bind_group,
            vr_mode: false,
        }
    }
    
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.size = (width, height);
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }
    
    /// Toggle VR mode on/off
    pub fn toggle_vr_mode(&mut self) {
        self.vr_mode = !self.vr_mode;
    }
    
    /// Render the scene with head tracking orientation
    pub fn render(&mut self, head_orientation: Quat) {
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(_) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        // Clear the screen
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        
        if self.vr_mode {
            // Stereoscopic rendering - render left and right eyes
            self.render_eye(&mut encoder, &view, head_orientation, -Self::IPD / 2.0, 0); // Left
            self.render_eye(&mut encoder, &view, head_orientation, Self::IPD / 2.0, 1);  // Right
        } else {
            // Normal mono rendering
            self.render_eye(&mut encoder, &view, head_orientation, 0.0, 2); // Center (full screen)
        }
        
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
    
    /// Render one eye's view
    fn render_eye(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        head_orientation: Quat,
        eye_offset: f32,
        eye_index: u32, // 0=left, 1=right, 2=center
    ) {
        let (width, height) = self.size;
        
        // Calculate viewport based on eye
        let (viewport_x, viewport_width) = match eye_index {
            0 => (0, width / 2),           // Left eye
            1 => (width / 2, width / 2),   // Right eye
            _ => (0, width),               // Center (mono)
        };
        
        // Create view matrix from head orientation
        let view_matrix = Mat4::from_quat(head_orientation.inverse());
        
        // Create perspective projection
        let aspect = viewport_width as f32 / height as f32;
        let fov_y = 90.0_f32.to_radians(); // Wide FOV for VR
        let proj_matrix = Mat4::perspective_rh(fov_y, aspect, 0.1, 100.0);
        
        // Combine view and projection
        let view_proj = proj_matrix * view_matrix;
        
        // Update camera uniforms
        let camera_uniforms = CameraUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            eye_offset: [eye_offset, if self.vr_mode { 1.0 } else { 0.0 }, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniforms));
        
        // Render pass for this eye
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Eye Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Don't clear, preserve previous content
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            render_pass.set_viewport(
                viewport_x as f32,
                0.0,
                viewport_width as f32,
                height as f32,
                0.0,
                1.0,
            );
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
    }
}
