//! VR Core - Main library for the Vision Pro-style Android VR app
//!
//! This module initializes the wgpu renderer, handles input from PS5 controllers,
//! and manages floating windows for web content.

use android_activity::AndroidApp;
use log::info;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{TouchPhase, WindowEvent};
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
mod thumbs;
mod webview;

/// Content zoom limits. The old 3.0 ceiling made the screen stop growing well
/// before it filled the field of view on a wide panel; 8.0 lets it go properly
/// cinema-sized, and the lower bound gets a bit more room to pull back too.
const ZOOM_MIN: f32 = 0.3;
const ZOOM_MAX: f32 = 8.0;

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
    /// Previous R2 analog value, for edge-detecting the browser click.
    prev_r2: f32,
    // Evdev Gamepad Reader
    #[allow(dead_code)]
    gamepad_reader: Option<gamepad::GamepadReader>,
    // Stereoscopic 3D layout for video: 0 = mono/2D, 1 = side-by-side, 2 = over-under.
    #[allow(dead_code)]
    stereo_mode: u32,
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
            prev_r2: 0.0,
            gamepad_reader: Some(gamepad::GamepadReader::new()),
            stereo_mode: 0,
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
        _window_id: WindowId,
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
                    let mut raw_input = state.take_egui_input(window);
                    // The UI is rasterized into a FIXED 2048x2048 SQUARE texture that gets
                    // curved onto the centered VR panel (renderer.rs render_eye + ScreenDescriptor
                    // { size_in_pixels: [2048, 2048] }, ui_panel.wgsl). egui_winit otherwise
                    // derives screen_rect from the real (non-square, e.g. wide) device window,
                    // so the dock/Media Center get laid out/centered against the WIDE window
                    // rect but rasterized onto a SQUARE canvas - content anchored toward the
                    // real window's right/top edge lands outside or in a corner of the square.
                    // Lock layout to the same square space that gets rasterized so panels are
                    // actually centered where they're drawn.
                    state.egui_ctx().set_pixels_per_point(1.0);
                    raw_input.screen_rect = Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(2048.0, 2048.0),
                    ));
                    state.egui_ctx().begin_pass(raw_input);

                    // Media Center thumbnails (hardware-accelerated): upload finished
                    // posters as GPU textures, then request posters for new video tiles.
                    if ui.media_visible() {
                        let ctx = state.egui_ctx();
                        for t in thumbs::drain() {
                            let img = egui::ColorImage::from_rgba_unmultiplied(
                                [t.w as usize, t.h as usize], &t.rgba);
                            let tex = ctx.load_texture(
                                format!("thumb:{}", t.path), img, egui::TextureOptions::LINEAR);
                            ui.file_browser.set_thumbnail(
                                std::path::Path::new(&t.path), tex, t.glow);
                        }
                        for path in ui.file_browser.pending_thumbnail_requests(12) {
                            thumbs::request(&self.app, &path.to_string_lossy(), 320, 180);
                        }
                    }

                    ui.render(state.egui_ctx(), self.renderer.as_ref().map(|r| r.vr_mode).unwrap_or(false));
                    
                    let output = state.egui_ctx().end_pass();
                    
                    state.handle_platform_output(window, output.platform_output.clone());
                    
                    full_output = Some(output);
                    ctx_clone = Some(state.egui_ctx().clone());
                    
                    // ── Browser tab model: pull Java's view a few times a second ──
                    //
                    // OWNERSHIP: Gecko (Java) owns the real sessions and is the sole
                    // authority on tab count / active tab / per-tab url, title and
                    // aspect. Rust keeps a read-only MIRROR, refreshed only here, and
                    // never mutates it directly — every change goes out as an intent
                    // and comes back on the next sync.
                    if ui.params.web_mode {
                        ui.web_browser.info_tick = ui.web_browser.info_tick.wrapping_add(1);
                        if ui.web_browser.info_tick % 12 == 0 {
                            if let Some(snap) = webview::tab_snapshot(&self.app) {
                                ui.sync_tabs(snap);
                            }
                        }
                    }

                    // ── Drain one-shot intents (see ui::Intent) ───────────────
                    for intent in ui.drain_intents() {
                        use ui::Intent as I;
                        match intent {
                            I::Recenter => {
                                if let Some(sensors) = &self.sensors { sensors.recenter(); }
                            }
                            I::ExitVr => {
                                if let Some(renderer) = &mut self.renderer {
                                    renderer.vr_mode = false;
                                    info!("Exited VR Mode via Menu");
                                }
                            }
                            I::PlayFile(path) => {
                                let path_str = path.to_string_lossy().to_string();
                                info!("Media Center: play {}", path_str);
                                // The fragment shader draws the browser texture in
                                // preference to video, so web mode has to go.
                                if ui.params.web_mode {
                                    ui.params.web_mode = false;
                                    ui.menu_state = ui::MenuState::Main;
                                }
                                if let Some(decoder) = &mut self.ndk_decoder { decoder.stop(); }
                                video::start_audio_from_path(&self.app, &path_str);
                                if let Ok(file) = std::fs::File::open(&path) {
                                    use std::os::unix::io::AsRawFd;
                                    let fd = file.as_raw_fd();
                                    let mut decoder = video_ndk::NdkVideoDecoder::new();
                                    if decoder.start_from_fd(fd).is_ok() {
                                        self.ndk_decoder = Some(decoder);
                                        info!("Started playback: {}", path_str);
                                    }
                                    // The decoder owns the FD for its lifetime.
                                    std::mem::forget(file);
                                }
                            }
                            I::TogglePlayPause => toggle_playback(&self.app, &self.ndk_decoder),
                            I::Seek(us) => seek_relative(&self.app, &self.ndk_decoder, us),
                            I::SetEngine(e)    => webview::set_engine(&self.app, e),
                            I::Navigate(url)   => {
                                webview::load_url(&self.app, &url);
                                ui.web_browser.current_url = url;
                            }
                            I::BrowserBack     => webview::go_back(&self.app),
                            I::BrowserForward  => webview::go_forward(&self.app),
                            I::BrowserReload   => webview::reload(&self.app),
                            I::NewTab          => webview::new_tab(&self.app),
                            I::CloseTabAt(i)   => webview::close_tab_at(&self.app, i as i32),
                            I::SelectTab(i)    => webview::select_tab(&self.app, i as i32),
                            I::SwitchTab(d)    => webview::switch_tab(&self.app, d),
                            I::CycleAspect     => webview::cycle_aspect(&self.app),
                            I::Tap(x, y)       => webview::tap(&self.app, x, y),
                            I::Scroll(dx, dy, x, y) => webview::inject_scroll(&self.app, dx, dy, x, y),
                            I::TypeText(t)     => webview::type_text(&self.app, &t),
                            I::SubmitEnter     => webview::submit_enter(&self.app),
                        }
                    }

                    // Publish the audio clock so the video decoder can pace against
                    // it. This is what actually keeps A/V in sync.
                    if let Some(decoder) = &self.ndk_decoder {
                        if decoder.is_running() && !decoder.is_paused() {
                            let ms = video::audio_position_ms(&self.app);
                            decoder.set_audio_clock_us(if ms < 0 { -1 } else { ms as i64 * 1000 });
                        } else {
                            decoder.set_audio_clock_us(-1);
                        }
                    }

                    // ── Gamepad (polled once per frame) ───────────────────────
                    let gp = gamepad::poll_actions();

                    // R2 is an ANALOG axis on a DualSense; edge-detect it so a held
                    // trigger clicks once rather than every frame.
                    let r2_edge = gp.r2_trigger > 0.55 && self.prev_r2 <= 0.55;
                    self.prev_r2 = gp.r2_trigger;

                    // Always-active, focus-independent.
                    if gp.reset_view {
                        if let Some(sensors) = &self.sensors { sensors.recenter(); }
                    }
                    if gp.toggle_vr_mode {
                        if let Some(renderer) = &mut self.renderer {
                            renderer.vr_mode = !renderer.vr_mode;
                        }
                    }

                    // ── Modal dispatch on the focus state machine ─────────────
                    // Exactly one arm runs, so no two surfaces can claim a button.
                    match ui.focus() {
                        ui::Focus::Keyboard => {
                            // D-pad picks a key, X types it, O backspaces,
                            // Options submits, △ dismisses.
                            if gp.nav_left  { ui.keyboard.move_left(); }
                            if gp.nav_right { ui.keyboard.move_right(); }
                            if gp.nav_up    { ui.keyboard.move_up(); }
                            if gp.nav_down  { ui.keyboard.move_down(); }
                            if gp.play_pause || gp.confirm { ui.keyboard.press(); }
                            if gp.back          { ui.keyboard.backspace(); }
                            if gp.open_settings { ui.keyboard_commit(); }
                            if gp.toggle_ui     { let b = ui.base_focus(); ui.set_focus(b); }
                        }
                        ui::Focus::MediaCenter => {
                            // Left-stick coverflow sweep + D-pad; X open; O up a level.
                            ui.file_browser.handle_stick(gp.left_stick_x);
                            if gp.nav_up   || gp.nav_left  { ui.file_browser.move_up(); }
                            if gp.nav_down || gp.nav_right { ui.file_browser.move_down(); }
                            if gp.play_pause || gp.confirm { ui.media_select(); }
                            if gp.back { ui.file_browser.go_back(); }
                            // Create closes it again (it opened it), Options swaps to the dock.
                            if gp.open_file_picker { let b = ui.base_focus(); ui.set_focus(b); }
                            if gp.open_settings    { ui.set_focus(ui::Focus::Dock); }
                            if gp.toggle_ui        { ui.set_focus(ui::Focus::Keyboard); }
                        }
                        ui::Focus::Dock => {
                            // D-pad left/right move the highlight, X activates, Options/O close.
                            if gp.nav_left  { ui.dock_move_left(); }
                            if gp.nav_right { ui.dock_move_right(); }
                            if gp.play_pause || gp.confirm { ui.dock_activate(); }
                            if gp.back || gp.open_settings { let b = ui.base_focus(); ui.set_focus(b); }
                            if gp.toggle_ui { ui.set_focus(ui::Focus::Keyboard); }
                        }
                        ui::Focus::TabOverview => {
                            if gp.nav_left  { ui.web_browser.overview_move(-1); }
                            if gp.nav_right { ui.web_browser.overview_move(1); }
                            if gp.nav_up    { ui.web_browser.overview_move(-2); }
                            if gp.nav_down  { ui.web_browser.overview_move(2); }
                            if gp.play_pause {
                                let i = ui.web_browser.overview_sel;
                                ui.push(ui::Intent::SelectTab(i));
                                ui.set_focus(ui::Focus::Browser);
                            }
                            if gp.back {
                                let i = ui.web_browser.overview_sel;
                                ui.push(ui::Intent::CloseTabAt(i));
                            }
                            if gp.confirm || gp.toggle_ui { ui.set_focus(ui::Focus::Browser); }
                            if gp.open_settings { ui.set_focus(ui::Focus::Dock); }
                        }
                        ui::Focus::Browser => {
                            // Stick cursor: RIGHT stick moves it, LEFT stick scrolls.
                            ui.web_browser.move_cursor(gp.right_stick_x, gp.right_stick_y);
                            ui.browser_scroll(gp.left_stick_x, gp.left_stick_y);
                            if r2_edge || gp.play_pause { ui.browser_click(); }
                            if gp.seek_back    { ui.push(ui::Intent::SwitchTab(-1)); }
                            if gp.seek_forward { ui.push(ui::Intent::SwitchTab(1)); }
                            if gp.back         { ui.push(ui::Intent::BrowserBack); }
                            if gp.confirm {
                                ui.web_browser.overview_sel = ui.web_browser.active_tab;
                                ui.set_focus(ui::Focus::TabOverview);
                            }
                            if gp.toggle_ui     { ui.toggle_focus(ui::Focus::Keyboard); }
                            if gp.open_settings { ui.toggle_focus(ui::Focus::Dock); }
                            if gp.open_file_picker {
                                ui.file_browser.refresh_entries();
                                ui.toggle_focus(ui::Focus::MediaCenter);
                            }
                            // Per-tab viewport shape (the video-mode stereo binding
                            // lives on the same key but only in Focus::Video).
                            if gp.nav_right { ui.push(ui::Intent::CycleAspect); }
                            if gp.nav_left  { ui.push(ui::Intent::NewTab); }
                            // Global geometry / head-tracking bindings stay live.
                            if gp.nav_up   { cycle_projection(ui); }
                            if gp.nav_down {
                                sensors::cycle_head_mode();
                                info!("Head tracking mode -> {}", sensors::head_mode_label());
                            }
                        }
                        ui::Focus::Video => {
                            // No panel: Options opens the dock, Create the Media Center,
                            // △ the keyboard, X play/pause, L1/R1 seek, D-pad L/R 3D layout.
                            if gp.open_settings { ui.toggle_focus(ui::Focus::Dock); }
                            if gp.toggle_ui     { ui.toggle_focus(ui::Focus::Keyboard); }
                            if gp.open_file_picker {
                                ui.file_browser.refresh_entries();
                                ui.toggle_focus(ui::Focus::MediaCenter);
                            }
                            if gp.play_pause   { toggle_playback(&self.app, &self.ndk_decoder); }
                            if gp.seek_back    { seek_relative(&self.app, &self.ndk_decoder, -10_000_000); }
                            if gp.seek_forward { seek_relative(&self.app, &self.ndk_decoder, 10_000_000); }
                            if gp.nav_right {
                                ui.params.stereo_mode = (ui.params.stereo_mode + 1) % ui::STEREO_MODES;
                                info!("3D -> {}", ui::stereo_label(ui.params.stereo_mode));
                            }
                            if gp.nav_left {
                                ui.params.stereo_mode =
                                    (ui.params.stereo_mode + ui::STEREO_MODES - 1) % ui::STEREO_MODES;
                                info!("3D -> {}", ui::stereo_label(ui.params.stereo_mode));
                            }
                            if gp.nav_up   { cycle_projection(ui); }
                            if gp.nav_down {
                                sensors::cycle_head_mode();
                                info!("Head tracking mode -> {}", sensors::head_mode_label());
                            }
                        }
                    }

                    // Zoom (L2/R2 analog). Suppressed while the browser has focus —
                    // there R2 is the click button.
                    const TRIGGER_DEADZONE: f32 = 0.08;
                    const ZOOM_SPEED: f32 = 0.05;
                    if ui.focus() != ui::Focus::Browser && gp.r2_trigger > TRIGGER_DEADZONE {
                        ui.params.content_scale =
                            (ui.params.content_scale + ZOOM_SPEED * gp.r2_trigger).min(ZOOM_MAX);
                    }
                    if gp.l2_trigger > TRIGGER_DEADZONE {
                        ui.params.content_scale =
                            (ui.params.content_scale - ZOOM_SPEED * gp.l2_trigger).max(ZOOM_MIN);
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
                    }

                    // Browser: when in web mode, show the live page on the screen.
                    let web_mode = self.vr_ui.as_ref().map(|u| u.params.web_mode).unwrap_or(false);
                    if web_mode {
                        if let Some((w, h, rgba)) = webview::get_frame() {
                            renderer.update_web_texture(&rgba, w, h);
                        }
                    } else {
                        renderer.has_web = false;
                    }

                    renderer.stereo_mode = self.vr_ui.as_ref()
                        .map(|u| u.params.stereo_mode as u32).unwrap_or(0);
                    renderer.projection_mode = self.vr_ui.as_ref()
                        .map(|u| u.params.projection_mode as u32).unwrap_or(0);
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
                use winit::keyboard::PhysicalKey;
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
                                    .clamp(ZOOM_MIN, ZOOM_MAX);
                                
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

/// D-pad up: cycle the screen geometry (flat → 180 → 360 → vertical).
///
/// Entering a dome from mono: VR180/VR360 footage is overwhelmingly side-by-side
/// stereo and the per-eye split lives in `stereo_mode` (independent of projection),
/// so switching to a dome while still in 2D showed the whole packed frame to both
/// eyes. Default to SBS; D-pad left/right still overrides.
fn cycle_projection(ui: &mut ui::VrUi) {
    ui.params.projection_mode = (ui.params.projection_mode + 1) % ui::PROJECTION_MODES;
    let dome = matches!(ui.params.projection_mode, 1 | 2);
    if dome && ui.params.stereo_mode == 0 {
        ui.params.stereo_mode = 1;
        info!("Dome: auto-enabling side-by-side");
    }
    info!("Projection -> {} ({})",
        ui::projection_label(ui.params.projection_mode),
        ui::stereo_label(ui.params.stereo_mode));
}

/// Toggle play/pause for BOTH pipelines.
///
/// Video is decoded natively (NdkVideoDecoder) while audio runs on a Java
/// MediaPlayer, so a pause that only touched the decoder left the sound
/// playing on. Every transport control has to drive both halves.
fn toggle_playback(app: &android_activity::AndroidApp, decoder: &Option<video_ndk::NdkVideoDecoder>) {
    let Some(decoder) = decoder else { return };
    if decoder.is_paused() {
        decoder.resume();
        video::resume_audio(app);
        info!("Playback resumed (video + audio)");
    } else {
        decoder.pause();
        video::pause_audio(app);
        info!("Playback paused (video + audio)");
    }
}

/// Seek both pipelines by `delta_us`, clamped at zero.
fn seek_relative(
    app: &android_activity::AndroidApp,
    decoder: &Option<video_ndk::NdkVideoDecoder>,
    delta_us: i64,
) {
    let Some(decoder) = decoder else { return };
    let target = (decoder.get_position() + delta_us).max(0);
    decoder.seek(target);
    // Move the audio too, then let the decoder re-converge on the new audio clock.
    video::seek_audio(app, (target / 1000) as i32);
    info!("Seek {:+}s -> {}ms", delta_us / 1_000_000, target / 1000);
}
