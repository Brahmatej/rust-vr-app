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
mod video_ndk;
mod gamepad;

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
    // NDK Video Decoder
    ndk_decoder: Option<video_ndk::NdkVideoDecoder>,
    // Evdev Gamepad Reader
    gamepad_reader: Option<gamepad::GamepadReader>,
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
            ndk_decoder: None,
            gamepad_reader: Some(gamepad::GamepadReader::new()),
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
                
                // Check for pending video FD from file picker
                if let Some(fd) = video::get_pending_fd() {
                    info!("Got pending video FD: {}, starting NDK decoder", fd);
                    // Stop existing decoder if any
                    if let Some(mut old_decoder) = self.ndk_decoder.take() {
                        old_decoder.stop();
                    }
                    // Start new decoder with the FD
                    let mut decoder = video_ndk::NdkVideoDecoder::new();
                    if let Err(e) = decoder.start_from_fd(fd) {
                        log::error!("Failed to start decoder from FD: {}", e);
                    }
                    self.ndk_decoder = Some(decoder);
                }
                
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
                    
                    // 6. Handle Playback Controls (from UI buttons)
                    if ui.params.toggle_play_pause {
                        if let Some(decoder) = &self.ndk_decoder {
                            if decoder.is_paused() {
                                decoder.resume();
                                info!("Video Resumed");
                            } else {
                                decoder.pause();
                                info!("Video Paused");
                            }
                        }
                        ui.params.toggle_play_pause = false;
                    }
                    
                    if ui.params.seek_forward_flag {
                        if let Some(decoder) = &self.ndk_decoder {
                            let pos = decoder.get_position();
                            decoder.seek(pos + 10_000_000); // +10 seconds
                            info!("Seek Forward +10s");
                        }
                        ui.params.seek_forward_flag = false;
                    }
                    
                    if ui.params.seek_backward_flag {
                        if let Some(decoder) = &self.ndk_decoder {
                            let pos = decoder.get_position();
                            decoder.seek((pos - 10_000_000).max(0)); // -10 seconds
                            info!("Seek Backward -10s");
                        }
                        ui.params.seek_backward_flag = false;
                    }
                    
                    // 7. Handle Gamepad Actions (poll once per frame)
                    let gp_actions = gamepad::poll_actions();
                    
                    // Play/Pause (X button)
                    if gp_actions.play_pause {
                        if let Some(decoder) = &self.ndk_decoder {
                            if decoder.is_paused() {
                                decoder.resume();
                                info!("Gamepad: Play");
                            } else {
                                decoder.pause();
                                info!("Gamepad: Pause");
                            }
                        }
                    }
                    
                    // Seek (L1/R1)
                    if gp_actions.seek_back {
                        if let Some(decoder) = &self.ndk_decoder {
                            let pos = decoder.get_position();
                            decoder.seek((pos - 10_000_000).max(0));
                            info!("Gamepad: Seek -10s");
                        }
                    }
                    if gp_actions.seek_forward {
                        if let Some(decoder) = &self.ndk_decoder {
                            let pos = decoder.get_position();
                            decoder.seek(pos + 10_000_000);
                            info!("Gamepad: Seek +10s");
                        }
                    }
                    
                    // Toggle UI (△)
                    if gp_actions.toggle_ui {
                        ui.toggle_hamburger();
                        info!("Gamepad: Toggle UI");
                    }
                    
                    // Reset view/recenter (L3)
                    if gp_actions.reset_view {
                        if let Some(sensors) = &self.sensors {
                            sensors.recenter();
                            info!("Gamepad: Recenter View");
                        }
                    }
                    
                    // Toggle VR mode (R3)
                    if gp_actions.toggle_vr_mode {
                        if let Some(renderer) = &mut self.renderer {
                            renderer.vr_mode = !renderer.vr_mode;
                            info!("Gamepad: VR Mode = {}", renderer.vr_mode);
                        }
                    }
                    
                    // File browser controls
                    if ui.file_browser.visible {
                        // D-pad up/down OR L1/R1 = navigate list
                        if gp_actions.nav_up || gp_actions.seek_back {
                            ui.file_browser.move_up();
                        }
                        if gp_actions.nav_down || gp_actions.seek_forward {
                            ui.file_browser.move_down();
                        }
                        // X button = select in file browser
                        if gp_actions.play_pause {
                            ui.file_browser.select_current();
                            info!("Gamepad: File Browser Select");
                        }
                        // ○ button = go back in file browser
                        if gp_actions.back {
                            ui.file_browser.go_back();
                            info!("Gamepad: File Browser Back");
                        }
                        // △ button = close file browser
                        if gp_actions.toggle_ui {
                            ui.file_browser.visible = false;
                            info!("Gamepad: File Browser Close");
                        }
                    } else {
                        // Normal controls when file browser is closed
                        
                        // Open file browser (Create button)
                        if gp_actions.open_file_picker {
                            ui.file_browser.visible = true;
                            ui.file_browser.refresh_entries();
                            info!("Gamepad: Open File Browser");
                        }
                        
                        // Back/Close menu (○ button)
                        if gp_actions.back {
                            if ui.is_hamburger_visible() {
                                ui.toggle_hamburger();
                                info!("Gamepad: Close Menu");
                            }
                        }
                    }
                    
                    // Zoom controls (L2/R2 - always active)
                    if gp_actions.zoom_in {  // R2
                        ui.params.content_scale = (ui.params.content_scale + 0.02).min(3.0);
                    }
                    if gp_actions.zoom_out {  // L2
                        ui.params.content_scale = (ui.params.content_scale - 0.02).max(0.5);
                    }
                    
                    // D-pad volume controls (when D-pad events work)
                    // Left = volume down, Right = volume up
                    // Note: D-pad on PS5 sends MotionEvents, need to handle in nav actions
                    
                    // Check if a file was selected from browser
                    if let Some(selected_path) = ui.file_browser.take_selected_file() {
                        let path_str = selected_path.to_string_lossy().to_string();
                        info!("File Browser: Selected {}", path_str);
                        
                        // Start playing the selected video file
                        if let Some(decoder) = &mut self.ndk_decoder {
                            decoder.stop();
                        }
                        
                        // Start audio playback via Java MediaPlayer
                        video::start_audio_from_path(&self.app, &path_str);
                        
                        // Open the file and get FD for video decoder
                        if let Ok(file) = std::fs::File::open(&selected_path) {
                            use std::os::unix::io::AsRawFd;
                            let fd = file.as_raw_fd();
                            
                            // Create new decoder with file
                            let mut decoder = video_ndk::NdkVideoDecoder::new();
                            if decoder.start_from_fd(fd).is_ok() {
                                self.ndk_decoder = Some(decoder);
                                info!("Started playback: {}", path_str);
                            }
                            // Keep file open (leak it for now - decoder needs the FD)
                            std::mem::forget(file);
                        }
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
                    
                    // Fetch video frame from NDK decoder (Y+UV planes)
                    if let Some(decoder) = &self.ndk_decoder {
                        if let Some((y_data, uv_data, width, height)) = decoder.get_frame() {
                            if !y_data.is_empty() {
                                renderer.update_video_texture(&y_data, &uv_data, width, height);
                            }
                        }
                    } else if let Some(frame) = video::VideoManager::get_video_frame(&self.app) {
                        // Fallback path for Java-based video (not used with NDK decoder)
                        let _ = frame; // NDK path is preferred
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
            WindowEvent::KeyboardInput { event, .. } => {
                // Map gamepad button events to GamepadState
                use winit::keyboard::{KeyCode, PhysicalKey};
                use winit::event::ElementState;
                
                let pressed = event.state == ElementState::Pressed;
                
                // Extract Android keycode from physical_key
                if let PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Android(code)) = event.physical_key {
                    // Android gamepad keycodes
                    match code {
                        96 => { // BUTTON_A = X
                            gamepad::handle_button(96, pressed);
                            info!("GAMEPAD: X button {}", if pressed { "pressed" } else { "released" });
                        }
                        97 => { // BUTTON_B = ○
                            gamepad::handle_button(97, pressed);
                            info!("GAMEPAD: ○ button {}", if pressed { "pressed" } else { "released" });
                        }
                        99 => { // BUTTON_X = □
                            gamepad::handle_button(99, pressed);
                            info!("GAMEPAD: □ button {}", if pressed { "pressed" } else { "released" });
                        }
                        100 => { // BUTTON_Y = △
                            gamepad::handle_button(100, pressed);
                            info!("GAMEPAD: △ button {}", if pressed { "pressed" } else { "released" });
                        }
                        102 => { // BUTTON_L1 - Volume Down
                            gamepad::handle_button(102, pressed);
                            if pressed {
                                video::volume_down(&self.app);
                            }
                            info!("GAMEPAD: L1 button {} (Volume Down)", if pressed { "pressed" } else { "released" });
                        }
                        103 => { // BUTTON_R1 - Volume Up
                            gamepad::handle_button(103, pressed);
                            if pressed {
                                video::volume_up(&self.app);
                            }
                            info!("GAMEPAD: R1 button {} (Volume Up)", if pressed { "pressed" } else { "released" });
                        }
                        104 => { // BUTTON_L2
                            gamepad::handle_button(104, pressed);
                            info!("GAMEPAD: L2 button {}", if pressed { "pressed" } else { "released" });
                        }
                        105 => { // BUTTON_R2
                            gamepad::handle_button(105, pressed);
                            info!("GAMEPAD: R2 button {}", if pressed { "pressed" } else { "released" });
                        }
                        106 => { // BUTTON_THUMBL = L3
                            gamepad::handle_button(106, pressed);
                            info!("GAMEPAD: L3 button {}", if pressed { "pressed" } else { "released" });
                        }
                        107 => { // BUTTON_THUMBR = R3
                            gamepad::handle_button(107, pressed);
                            info!("GAMEPAD: R3 button {}", if pressed { "pressed" } else { "released" });
                        }
                        108 => { // BUTTON_START = Options
                            gamepad::handle_button(108, pressed);
                            info!("GAMEPAD: Options button {}", if pressed { "pressed" } else { "released" });
                        }
                        109 => { // BUTTON_SELECT = Create
                            gamepad::handle_button(109, pressed);
                            info!("GAMEPAD: Create button {}", if pressed { "pressed" } else { "released" });
                        }
                        110 => { // BUTTON_MODE = PS button
                            gamepad::handle_button(110, pressed);
                            info!("GAMEPAD: PS button {}", if pressed { "pressed" } else { "released" });
                        }
                        19 => { // DPAD_UP
                            gamepad::handle_button(19, pressed);
                            info!("GAMEPAD: D-pad UP {}", if pressed { "pressed" } else { "released" });
                        }
                        20 => { // DPAD_DOWN
                            gamepad::handle_button(20, pressed);
                            info!("GAMEPAD: D-pad DOWN {}", if pressed { "pressed" } else { "released" });
                        }
                        21 => { // DPAD_LEFT - Volume Down
                            gamepad::handle_button(21, pressed);
                            if pressed {
                                video::volume_down(&self.app);
                            }
                            info!("GAMEPAD: D-pad LEFT {} (Volume Down)", if pressed { "pressed" } else { "released" });
                        }
                        22 => { // DPAD_RIGHT - Volume Up
                            gamepad::handle_button(22, pressed);
                            if pressed {
                                video::volume_up(&self.app);
                            }
                            info!("GAMEPAD: D-pad RIGHT {} (Volume Up)", if pressed { "pressed" } else { "released" });
                        }
                        _ => {
                            info!("GAMEPAD: Unknown button code={} {}", code, if pressed { "pressed" } else { "released" });
                        }
                    }
                }
            }
            WindowEvent::Touch(touch) => {
                let id = touch.id;
                let loc = (touch.location.x, touch.location.y);
                
                match touch.phase {
                    TouchPhase::Started => {
                        self.touches.insert(id, loc);
                        
                        // Show hamburger on any tap (resets auto-hide timer)
                        if let Some(ui) = &mut self.vr_ui {
                            ui.show_hamburger();
                        }
                        
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
