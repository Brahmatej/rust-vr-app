//! VR Core - Main library for the Vision Pro-style Android VR app
//!
//! This module initializes the wgpu renderer, handles input from PS5 controllers,
//! and manages floating windows for web content.

use android_activity::{AndroidApp, MainEvent, PollEvent};
use log::info;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::window::{Window, WindowId};

mod renderer;
mod input;
mod window_manager;

/// Main application state
struct VRApp {
    window: Option<Arc<Window>>,
    renderer: Option<renderer::Renderer>,
}

impl VRApp {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
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
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        info!("App suspended - releasing GPU resources");
        self.renderer = None;
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
                if let Some(renderer) = &mut self.renderer {
                    renderer.render();
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
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
