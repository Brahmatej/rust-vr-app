//! VR Core - Main library for the Vision Pro-style Android VR app
//!
//! This module initializes the wgpu renderer, handles input from PS5 controllers,
//! and manages floating windows for web content.

use android_activity::AndroidApp;
use log::info;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::window::{Window, WindowId};
use glam::Quat;

mod renderer;
mod input;
mod window_manager;
mod sensors;
mod ui;
mod video;

/// Main application state
struct VRApp {
    window: Option<Arc<Window>>,
    renderer: Option<renderer::Renderer>,
    sensors: Option<sensors::SensorInput>,
    last_frame_time: Instant,
    
    // UI State
    egui_state: Option<egui_winit::State>,
    vr_ui: Option<ui::VrUi>,
    app: AndroidApp,
    
    // Pinch-to-Zoom
    touches: std::collections::HashMap<u64, (f64, f64)>,
    initial_pinch_distance: Option<f64>,
    initial_content_scale: f32,
}

impl VRApp {
    fn new(app: AndroidApp) -> Self {
        Self {
            window: None,
            renderer: None,
            sensors: None,
            last_frame_time: Instant::now(),
            egui_state: None,
            vr_ui: None,
            app,
            touches: std::collections::HashMap::new(),
            initial_pinch_distance: None,
            initial_content_scale: 1.0,
        }
    }
}

impl ApplicationHandler for VRApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        info!("App resumed - creating window");
        
        let window_attrs = Window::default_attributes()
            .with_title("VR Space");
        
        let window = Arc::new(event_loop.create_window(window_attrs).unwrap());
        self.window = Some(window.clone());
        
        // Initialize wgpu renderer
        self.renderer = Some(pollster::block_on(renderer::Renderer::new(window.clone())));
        info!("Renderer initialized");
        
        // Initialize sensors only once (preserve across pause/resume)
        if self.sensors.is_none() {
            self.sensors = Some(sensors::SensorInput::new());
            if let Some(ref sensors) = self.sensors {
                if sensors.is_available() {
                    info!("Sensors available for head tracking");
                } else {
                    info!("No sensors available - using fixed orientation");
                }
            }
        } else {
            info!("Sensors preserved from previous session");
        }
        
        // Initialize UI
        let ctx = egui::Context::default();
        self.vr_ui = Some(ui::VrUi::new(&ctx));
        
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            None,
            None,
            None
        );
        self.egui_state = Some(state);
        
        self.last_frame_time = Instant::now();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        info!("App suspended - releasing GPU resources");
        self.renderer = None;
        self.sensors = None;
        self.window = None;
        self.egui_state = None;
        self.vr_ui = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Pass event to egui
        let response = if let (Some(state), Some(window)) = (&mut self.egui_state, &self.window) {
             state.on_window_event(window, &event)
        } else {
            Default::default()
        };
        
        if response.consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                info!("Close requested");
                event_loop.exit();
            }
            
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame_time).as_secs_f32();
                self.last_frame_time = now;
                
                // UI Logic
                let mut full_output = None;
                let mut ctx_clone = None;
                
                if let (Some(state), Some(ui), Some(window)) = (&mut self.egui_state, &mut self.vr_ui, &self.window) {
                    let raw_input = state.take_egui_input(window);
                    state.egui_ctx().begin_frame(raw_input);
                    
                    ui.render(state.egui_ctx(), self.renderer.as_ref().map(|r| r.vr_mode).unwrap_or(false));
                    
                    let output = state.egui_ctx().end_frame();
                    
                    state.handle_platform_output(window, output.platform_output.clone());
                    
                    full_output = Some(output);
                    ctx_clone = Some(state.egui_ctx().clone());
                    
                    // Apply UI Params
                    // 1. Recenter
                    if ui.params.recenter_flag {
                         if let Some(sensors) = &self.sensors {
                            sensors.recenter();
                         }
                         ui.params.recenter_flag = false; // Reset flag
                    }
                    
                    // 2. Gyro Toggle (handled in update below)
                    // 3. Distortion (passed to renderer later)

                    // 4. Check Selection
                    if ui.params.select_video_flag {
                         info!("UI: Select Video Requested");
                         ui.params.select_video_flag = false;
                         video::VideoManager::pick_video(&self.app);
                    }
                    
                    // 5. Check Exit Request
                    if ui.params.vr_exit_requested {
                        if let Some(renderer) = &mut self.renderer {
                             renderer.vr_mode = false;
                             info!("Exited VR Mode via Menu");
                        }
                        ui.params.vr_exit_requested = false; // Reset flag
                    }
                }

                
                // Update sensors
                let orientation = if let Some(ui) = &self.vr_ui {
                    if ui.params.gyro_enabled {
                         if let Some(ref mut sensors) = self.sensors {
                            sensors.update(dt);
                            sensors.get_orientation()
                        } else {
                            Quat::IDENTITY
                        }
                    } else {
                        Quat::IDENTITY
                    }
                } else {
                     // Fallback if UI not ready
                     if let Some(ref mut sensors) = self.sensors {
                        sensors.update(dt);
                        sensors.get_orientation()
                    } else {
                        Quat::IDENTITY
                    }
                };
                
                // Render
                if let Some(renderer) = &mut self.renderer {
                    // Extract Distortion Params
                    let distortion_params = if let Some(ui) = &self.vr_ui {
                        Some((ui.params.lens_radius, ui.params.lens_center_offset))
                    } else {
                         Some((1.0, 0.0))
                    };
                    
                    // Construct UI data bundle
                    let ui_data = if let (Some(out), Some(ctx)) = (full_output, &ctx_clone) {
                        Some((ctx, out))
                    } else {
                        None
                    };

                    let content_scale = self.vr_ui.as_ref()
                        .map(|ui| ui.params.content_scale)
                        .unwrap_or(1.0);
                    
                    // Fetch video frame from Java and upload to GPU
                    if let Some(frame) = video::VideoManager::get_video_frame(&self.app) {
                        renderer.update_video_texture(&frame.data, frame.width, frame.height);
                    }
                        
                    renderer.render(orientation, ui_data, distortion_params, content_scale);
                }
                
                // Request next frame
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Touch(touch) => {
                let id = touch.id;
                let loc = (touch.location.x, touch.location.y);
                
                match touch.phase {
                    TouchPhase::Started => {
                        self.touches.insert(id, loc);
                        
                        // If 2 fingers touched, start pinch
                        if self.touches.len() == 2 {
                            let positions: Vec<_> = self.touches.values().collect();
                            let dx = positions[1].0 - positions[0].0;
                            let dy = positions[1].1 - positions[0].1;
                            self.initial_pinch_distance = Some((dx * dx + dy * dy).sqrt());
                            self.initial_content_scale = self.vr_ui.as_ref()
                                .map(|ui| ui.params.content_scale).unwrap_or(1.0);
                        }
                    }
                    TouchPhase::Moved => {
                        self.touches.insert(id, loc);
                        
                        // If 2 fingers, calculate zoom
                        if self.touches.len() == 2 {
                            if let Some(initial_dist) = self.initial_pinch_distance {
                                let positions: Vec<_> = self.touches.values().collect();
                                let dx = positions[1].0 - positions[0].0;
                                let dy = positions[1].1 - positions[0].1;
                                let current_dist = (dx * dx + dy * dy).sqrt();
                                
                                // Calculate zoom factor
                                let scale_factor = (current_dist / initial_dist) as f32;
                                let new_scale = (self.initial_content_scale * scale_factor)
                                    .clamp(0.5, 3.0);
                                
                                if let Some(ui) = &mut self.vr_ui {
                                    ui.params.content_scale = new_scale;
                                }
                            }
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.touches.remove(&id);
                        
                        // Reset pinch state
                        if self.touches.len() < 2 {
                            self.initial_pinch_distance = None;
                        }
                        
                        // VR toggle (single tap, non-VR mode only)
                        if self.touches.is_empty() && self.initial_pinch_distance.is_none() {
                            if let Some(renderer) = &mut self.renderer {
                                if !renderer.vr_mode {
                                    if let Some(window) = &self.window {
                                        let size = window.inner_size();
                                        if touch.location.y < (size.height as f64 * 0.7) {
                                            renderer.toggle_vr_mode();
                                            info!("Entered VR Mode");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            _ => {}
        }
    }
}

/// Android entry point
#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("VRApp"),
    );
    
    info!("VR App starting...");
    
    let event_loop = EventLoop::builder()
        .with_android_app(app.clone())
        .build()
        .expect("Failed to create event loop");
    
    let mut vr_app = VRApp::new(app);
    event_loop.run_app(&mut vr_app).expect("Event loop failed");
}
