//! wgpu Renderer module
//!
//! Handles GPU initialization and 3D rendering for the VR environment.
//! Supports both normal and stereoscopic VR rendering modes.

use std::sync::Arc;
use egui_wgpu::wgpu;
use wgpu::{
    Backends, Device, DeviceDescriptor, Instance, InstanceDescriptor, Queue,
    RenderPipeline, Surface, SurfaceConfiguration, SurfaceTargetUnsafe, TextureUsages,
    BindGroup, BindGroupLayout, Buffer,
};
use winit::window::Window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use glam::{Mat4, Quat, Vec3};
use bytemuck::{Pod, Zeroable};

// Camera uniforms
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
    eye_offset: [f32; 4], // x = eye offset, y = has_video, z = time, w = content_scale
    video_info: [f32; 4], // x = aspect_ratio, y = width, z = height, w = unused
}

// Distortion uniforms
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DistortionUniforms {
    lens_radius: f32,       // Circle size
    lens_center_offset: f32, // Horizontal shift
    scale_factor: f32,       // Dynamic zoom
    padding2: f32,
}

pub struct Renderer {
    #[allow(dead_code)]
    window: Arc<Window>,
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    
    // Main Scene
    pipeline: RenderPipeline,
    size: (u32, u32),
    camera_buffer: Buffer,
    camera_bind_group: BindGroup,
    #[allow(dead_code)]
    camera_bind_group_layout: BindGroupLayout,
    
    // Video Texture
    video_texture_y: Option<wgpu::Texture>,
    video_texture_y_view: Option<wgpu::TextureView>,
    video_texture_uv: Option<wgpu::Texture>,
    video_texture_uv_view: Option<wgpu::TextureView>,
    video_sampler: wgpu::Sampler,
    video_bind_group: wgpu::BindGroup,  // Always valid (placeholder or real)
    video_bind_group_layout: wgpu::BindGroupLayout,
    has_video: bool,
    video_width: u32,
    video_height: u32,
    
    // UI Texture (for Shader Overlay)
    ui_texture: wgpu::Texture,
    ui_texture_view: wgpu::TextureView,
    
    // UI Texture (for Shader Overlay)

    
    // Post Processing (Distortion)
    offscreen_texture: wgpu::Texture,
    offscreen_view: wgpu::TextureView,
    offscreen_sampler: wgpu::Sampler,
    distortion_pipeline: RenderPipeline,
    distortion_bind_group: BindGroup,
    distortion_bind_group_layout: BindGroupLayout,
    distortion_buffer: Buffer,
    
    // VR mode state
    pub vr_mode: bool,
    
    // UI Renderer
    egui_renderer: egui_wgpu::Renderer,
    
    // Animation
    start_time: std::time::Instant,
}

impl Renderer {
    // Inter-pupillary distance (average human IPD is ~63mm)
    const IPD: f32 = 0.063;
    
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::VULKAN,
            ..Default::default()
        });
        
        let surface = unsafe {
            let window_handle = window.window_handle().unwrap().as_raw();
            let display_handle = window.display_handle().unwrap().as_raw();
            let target = SurfaceTargetUnsafe::RawHandle { 
                raw_display_handle: display_handle, 
                raw_window_handle: window_handle,
            };
            instance.create_surface_unsafe(target).unwrap()
        };
        
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }).await.expect("Failed to find GPU adapter");
        
        let (device, queue) = adapter.request_device(&DeviceDescriptor::default(), None).await.expect("Failed to create device");
        
        // Log wgpu errors instead of panicking
        device.on_uncaptured_error(Box::new(|error| {
            log::error!("WGPU ERROR: {:?}", error);
        }));
        
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
        
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Buffer"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/main.wgsl").into()),
        });

        // Create egui renderer with Rgba8UnormSrgb to match ui_texture format
        let egui_renderer = egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, None, 1, false);

        // --- Video Texture Setup ---
        let video_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        
        let video_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Video Bind Group Layout"),
            entries: &[
                // Y texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // UV texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // UI Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
            ],
        });

        // Create Pipeline Layout (after video_bind_group_layout)
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout, &video_bind_group_layout],
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

        // Create UI Texture (Fixed high-res for VR clarity)
        let ui_texture_size = wgpu::Extent3d { width: 2048, height: 2048, depth_or_array_layers: 1 };
        let ui_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Texture"),
            size: ui_texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb, // Match egui pipeline format
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ui_texture_view = ui_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create placeholder 1x1 video textures (required for bind group)
        let placeholder_texture_y = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Placeholder Video Texture Y"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let placeholder_view_y = placeholder_texture_y.create_view(&wgpu::TextureViewDescriptor::default());

        let placeholder_texture_uv = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Placeholder Video Texture UV"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let placeholder_view_uv = placeholder_texture_uv.create_view(&wgpu::TextureViewDescriptor::default());

        let video_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video Bind Group (Placeholder)"),
            layout: &video_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&placeholder_view_y) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&placeholder_view_uv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&video_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&ui_texture_view) },
            ],
        });

        // --- Distortion Pipeline Setup ---
        
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Texture"),
            size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let offscreen_texture = device.create_texture(&texture_desc);
        let offscreen_view = offscreen_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let offscreen_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        
        let distortion_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Distortion Buffer"),
            size: std::mem::size_of::<DistortionUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let distortion_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Distortion Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        
        let distortion_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Distortion Bind Group"),
            layout: &distortion_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&offscreen_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&offscreen_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: distortion_buffer.as_entire_binding() },
            ],
        });
        
        let distortion_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Distortion Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/distortion.wgsl").into()),
        });
        
        let distortion_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Distortion Pipeline Layout"),
            bind_group_layouts: &[&distortion_bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let distortion_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Distortion Pipeline"),
            layout: Some(&distortion_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &distortion_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &distortion_shader,
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
            camera_bind_group_layout: bind_group_layout,
            
            // Video (placeholder initially)
            video_texture_y: None,
            video_texture_y_view: None,
            video_texture_uv: None,
            video_texture_uv_view: None,
            video_sampler,
            video_bind_group,
            video_bind_group_layout,
            has_video: false,
            video_width: 1920,  // Default 16:9
            video_height: 1080,
            
            vr_mode: false,
            egui_renderer,
            offscreen_texture,
            offscreen_view,
            offscreen_sampler,
            distortion_pipeline,

            distortion_bind_group,
            distortion_bind_group_layout,
            distortion_buffer,
            start_time: std::time::Instant::now(),
            ui_texture,
            ui_texture_view,
        }
    }
    
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.size = (width, height);
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            
            let texture_desc = wgpu::TextureDescriptor {
                label: Some("Offscreen Texture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };
            self.offscreen_texture = self.device.create_texture(&texture_desc);
            self.offscreen_view = self.offscreen_texture.create_view(&wgpu::TextureViewDescriptor::default());
            
            self.distortion_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Distortion Bind Group"),
                layout: &self.distortion_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.offscreen_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.offscreen_sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.distortion_buffer.as_entire_binding() },
                ],
            });
        }
    }
    
    pub fn toggle_vr_mode(&mut self) {
        self.vr_mode = !self.vr_mode;
    }
    
    /// Creates Y and UV textures (R8 and Rg8)
    fn create_video_texture(&mut self, width: u32, height: u32) {
        if self.video_texture_y.is_some() && self.video_width == width && self.video_height == height {
             return;
        }

        let texture_y = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video Texture Y"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        let texture_uv = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video Texture UV"),
            size: wgpu::Extent3d { width: width / 2, height: height / 2, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        let view_y = texture_y.create_view(&wgpu::TextureViewDescriptor::default());
        let view_uv = texture_uv.create_view(&wgpu::TextureViewDescriptor::default());
        
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video YUV Bind Group"),
            layout: &self.video_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view_y) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view_uv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.video_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.ui_texture_view) },
            ],
        });
        
        self.video_texture_y = Some(texture_y);
        self.video_texture_y_view = Some(view_y);
        self.video_texture_uv = Some(texture_uv);
        self.video_texture_uv_view = Some(view_uv);
        self.video_bind_group = bind_group;
        self.video_width = width;
        self.video_height = height;
    }
    
    /// Updates video texture with new frame data from Java
    pub fn update_video_texture(&mut self, y_data: &[u8], uv_data: &[u8], width: u32, height: u32) {
        if self.video_texture_y.is_none() || self.video_width != width || self.video_height != height {
            self.create_video_texture(width, height);
            self.has_video = true;
        }

        if let Some(ref texture_y) = self.video_texture_y {
            self.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: texture_y,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                y_data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }
        
        if let Some(ref texture_uv) = self.video_texture_uv {
            self.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: texture_uv,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                uv_data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(width), // UV width is width/2 but bytes per pixel is 2 (Rg8), so stride is same as Y plane width
                    rows_per_image: Some(height / 2),
                },
                wgpu::Extent3d { width: width / 2, height: height / 2, depth_or_array_layers: 1 },
            );
        }
    }
    
    pub fn render(
        &mut self, 
        head_orientation: Quat, 
        ui_data: Option<(&egui::Context, egui::FullOutput)>,
        distortion_params: Option<(f32, f32)>, // lens_radius, lens_center_offset
        content_scale: f32, // New scalar for virtual screen size
    ) {
        let lens_offset_val = distortion_params.map(|(_, offset)| offset).unwrap_or(0.0);
        let lens_radius_val = distortion_params.map(|(radius, _)| radius).unwrap_or(1.0);
        
        // Calculate Scale Factor (Cardboard style)
        let k1 = 0.25;
        let k2 = 0.15;
        // Clamp input radius for scaling calculation to prevent "infinite" zoom visual
        // Even if lens_radius is 1.5, we calculate scale based on max 1.2 to keep some border visible if desired, 
        // or let it fill. User said "increasing lens size is infinite", implies it fills too much.
        // Let's cap the effective r for scaling.
        let r = lens_radius_val.min(1.0); 
        let r2 = r * r;
        let distortion_at_max = 1.0 + k1 * r2 + k2 * r2 * r2;
        let scale_factor_val = 1.0 / distortion_at_max;
        
        if let Some((radius, offset)) = distortion_params {
            let uniforms = DistortionUniforms { 
                lens_radius: radius, 
                lens_center_offset: offset, 
                scale_factor: scale_factor_val, 
                padding2: 0.0 
            };
            self.queue.write_buffer(&self.distortion_buffer, 0, bytemuck::bytes_of(&uniforms));
        }

        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(_) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let target_view = if self.vr_mode { &self.offscreen_view } else { &view };
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        // 1. Render UI to Texture (Always render, even if empty/hidden, to clear texture)
        {
            // Clear UI Texture
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ui_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        
        if let Some((ctx, ref full_output)) = ui_data {
            // For ui_texture pass, use texture dimensions (2048x2048)
            // The direct screen pass later will use screen size
            let screen_descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [2048, 2048],
                pixels_per_point: ctx.pixels_per_point(),
            };
            
            let paint_jobs = ctx.tessellate(full_output.shapes.clone(), full_output.pixels_per_point);
            
            for (id, delta) in &full_output.textures_delta.set {
                self.egui_renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
            
            self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &paint_jobs,
                &screen_descriptor,
            );
            
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("UI Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ui_texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load, // Load the cleared transparent texture
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                
                let render_pass_static: &mut wgpu::RenderPass<'static> = unsafe { std::mem::transmute(&mut render_pass) };
                self.egui_renderer.render(render_pass_static, &paint_jobs, &screen_descriptor);
            }
            
            for id in &full_output.textures_delta.free {
                self.egui_renderer.free_texture(id);
            }
        }
        
        // 2. Clear Screen
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        
        // 3. Render 3D Scene
        if self.vr_mode {
            self.render_eye(&mut encoder, target_view, head_orientation, -Self::IPD / 2.0, 0, lens_offset_val, content_scale); 
            self.render_eye(&mut encoder, target_view, head_orientation, Self::IPD / 2.0, 1, lens_offset_val, content_scale);  
        } else {
            self.render_eye(&mut encoder, target_view, head_orientation, 0.0, 2, 0.0, content_scale); 
        }
        
        // 4. Distortion Pass
        if self.vr_mode {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Distortion Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            render_pass.set_pipeline(&self.distortion_pipeline);
            render_pass.set_bind_group(0, &self.distortion_bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }
        
        // 5. Direct Screen UI Overlay (after distortion/scene, overlays on top)
        if let Some((ctx, ref full_output)) = ui_data {
            let (width, height) = self.size;
            let screen_descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: ctx.pixels_per_point(),
            };
            
            // Re-tessellate for screen size (different from ui_texture size)
            let paint_jobs = ctx.tessellate(full_output.shapes.clone(), full_output.pixels_per_point);
            log::info!("UI Direct Screen Pass: {} paint jobs, screen {}x{}", paint_jobs.len(), width, height);
            
            // The textures were already updated in the first pass, just need to update buffers
            self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &paint_jobs,
                &screen_descriptor,
            );
            
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("UI Direct Screen Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view, // Render directly to swapchain
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load, // Preserve existing content
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                
                let render_pass_static: &mut wgpu::RenderPass<'static> = unsafe { std::mem::transmute(&mut render_pass) };
                self.egui_renderer.render(render_pass_static, &paint_jobs, &screen_descriptor);
            }
        }
        
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
    
    // --- Phase 9: Proven Asymmetric Projection ---
    fn render_eye(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, head_orientation: Quat, base_eye_offset: f32, eye_index: u32, lens_center_dist_offset: f32, content_scale: f32) {
         let (width, height) = self.size;
        let (viewport_x, viewport_width) = match eye_index {
            0 => (0, width / 2),
            1 => (width / 2, width / 2),
            _ => (0, width),
        };
        
        let shift_x = if eye_index == 0 {
            lens_center_dist_offset 
        } else {
            -lens_center_dist_offset 
        };
        
        let dynamic_offset = if eye_index == 0 {
            base_eye_offset + lens_center_dist_offset
        } else {
            base_eye_offset - lens_center_dist_offset
        };
        
        let near = 0.1;
        let far = 100.0;
        let fov_y_radians = 90.0_f32.to_radians();
        let aspect = viewport_width as f32 / height as f32;
        
        let top = near * (fov_y_radians / 2.0).tan();
        let bottom = -top;
        
        let half_width = top * aspect;
        let shift_near = shift_x * half_width * 2.0; 
        
        let left = -half_width - shift_near;
        let right = half_width - shift_near;
        
        let x_scale = 2.0 * near / (right - left);
        let y_scale = 2.0 * near / (top - bottom);
        let x_offset = (right + left) / (right - left);
        let y_offset = (top + bottom) / (top - bottom);
        let z_scale = far / (near - far); 
        let z_offset = near * far / (near - far);
        
        let proj_matrix = Mat4::from_cols(
            glam::Vec4::new(x_scale, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, y_scale, 0.0, 0.0),
            glam::Vec4::new(x_offset, y_offset, z_scale, -1.0), 
            glam::Vec4::new(0.0, 0.0, z_offset, 0.0),
        );

        let view_matrix = Mat4::from_quat(head_orientation.inverse());
        let view_proj = proj_matrix * view_matrix;
        
        let camera_uniforms = CameraUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            // Pass has_video in .y, Time in .z, Content Scale in .w
            eye_offset: [dynamic_offset, if self.has_video { 1.0 } else { 0.0 }, self.start_time.elapsed().as_secs_f32(), content_scale],
            // Video aspect ratio (width/height)
            video_info: [self.video_width as f32 / self.video_height as f32, self.video_width as f32, self.video_height as f32, 0.0],
        };
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniforms));
        
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Eye Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { 
                        load: wgpu::LoadOp::Load, 
                        store: wgpu::StoreOp::Store 
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_viewport(viewport_x as f32, 0.0, viewport_width as f32, height as f32, 0.0, 1.0);
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            // Always bind video texture (placeholder or real)
            render_pass.set_bind_group(1, &self.video_bind_group, &[]);
            
            render_pass.draw(0..6, 0..1);
        }
    }
}
