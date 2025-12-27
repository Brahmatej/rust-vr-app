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

/// Main application state
struct VRApp {
    window: Option<Arc<Window>>,
    renderer: Option<renderer::Renderer>,
    sensors: Option<sensors::SensorInput>,
    last_frame_time: Instant,
    
    // Touch state for VR toggle
    touch_active: bool,
}

impl VRApp {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            sensors: None,
            last_frame_time: Instant::now(),
            touch_active: false,
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
        self.renderer = Some(pollster::block_on(renderer::Renderer::new(window)));
        info!("Renderer initialized");
        
        // Initialize sensors
        self.sensors = Some(sensors::SensorInput::new());
        if let Some(ref sensors) = self.sensors {
            if sensors.is_available() {
                info!("Sensors available for head tracking");
            } else {
                info!("No sensors available - using fixed orientation");
            }
        }
        
        self.last_frame_time = Instant::now();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        info!("App suspended - releasing GPU resources");
        self.renderer = None;
        self.sensors = None;
        self.window = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("Close requested");
                event_loop.exit();
            }
            
            WindowEvent::RedrawRequested => {
                // Calculate delta time
                let now = Instant::now();
                let dt = (now - self.last_frame_time).as_secs_f32();
                self.last_frame_time = now;
                
                // Update sensors
                let orientation = if let Some(ref mut sensors) = self.sensors {
                    sensors.update(dt);
                    sensors.orientation
                } else {
                    Quat::IDENTITY
                };
                
                // Render with current head orientation
                if let Some(renderer) = &mut self.renderer {
                    renderer.render(orientation);
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
            
            // Handle touch input for VR toggle
            WindowEvent::Touch(Touch { phase, .. }) => {
                match phase {
                    TouchPhase::Started => {
                        self.touch_active = true;
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if self.touch_active {
                            // Toggle VR mode on tap release
                            if let Some(renderer) = &mut self.renderer {
                                renderer.toggle_vr_mode();
                                let mode = if renderer.vr_mode { "VR" } else { "Normal" };
                                info!("Switched to {} mode", mode);
                            }
                            self.touch_active = false;
                        }
                    }
                    _ => {}
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
        .with_android_app(app)
        .build()
        .expect("Failed to create event loop");
    
    let mut vr_app = VRApp::new();
    event_loop.run_app(&mut vr_app).expect("Event loop failed");
}
