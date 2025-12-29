//! Modular gamepad input for Android
//!
//! Captures PS5 DualSense controller input via winit KeyboardInput events.
//! Provides both raw GamepadState and high-level GamepadActions for app control.

use std::sync::{Arc, Mutex};
use log::info;
use lazy_static::lazy_static;

/// Raw gamepad button/axis state
#[derive(Debug, Clone, Default)]
pub struct GamepadState {
    // Buttons (Android KeyEvent keycodes)
    pub btn_south: bool,      // X on DualSense (96)
    pub btn_east: bool,       // ○ on DualSense (97)
    pub btn_north: bool,      // △ on DualSense (100)
    pub btn_west: bool,       // □ on DualSense (99)
    pub btn_l1: bool,         // L1 (102)
    pub btn_r1: bool,         // R1 (103)
    pub btn_l2: bool,         // L2 digital (104)
    pub btn_r2: bool,         // R2 digital (105)
    pub btn_select: bool,     // Create (109)
    pub btn_start: bool,      // Options (108)
    pub btn_mode: bool,       // PS button (110)
    pub btn_thumbl: bool,     // L3 (106)
    pub btn_thumbr: bool,     // R3 (107)
    pub btn_dpad_up: bool,    // D-pad up (19)
    pub btn_dpad_down: bool,  // D-pad down (20)
    pub btn_dpad_left: bool,  // D-pad left (21)
    pub btn_dpad_right: bool, // D-pad right (22)
    
    // Axes (not yet implemented via winit - needs motion events)
    pub left_stick_x: f32,
    pub left_stick_y: f32,
    pub right_stick_x: f32,
    pub right_stick_y: f32,
    pub l2_trigger: f32,
    pub r2_trigger: f32,
}

/// High-level app actions triggered by gamepad
/// Each action is a one-shot boolean that fires on button press
#[derive(Debug, Clone, Default)]
pub struct GamepadActions {
    // Media controls
    pub play_pause: bool,       // X button
    pub seek_back: bool,        // L1 - seek backward 10s
    pub seek_forward: bool,     // R1 - seek forward 10s
    
    // UI controls  
    pub toggle_ui: bool,        // △ - show/hide menu
    pub confirm: bool,          // □ - select/confirm
    pub back: bool,             // ○ - back/cancel
    
    // VR controls
    pub reset_view: bool,       // L3 - recenter orientation
    pub toggle_vr_mode: bool,   // R3 - switch VR/2D
    
    // App controls
    pub open_settings: bool,    // Options button
    pub open_file_picker: bool, // Create button
    pub exit_app: bool,         // PS button
    
    // Zoom (analog triggers - for now digital)
    pub zoom_in: bool,          // R2
    pub zoom_out: bool,         // L2
    
    // Navigation
    pub nav_up: bool,           // D-pad up
    pub nav_down: bool,         // D-pad down
}

// Global state
lazy_static! {
    static ref GAMEPAD_STATE: Arc<Mutex<GamepadState>> = Arc::new(Mutex::new(GamepadState::default()));
    static ref PREV_STATE: Arc<Mutex<GamepadState>> = Arc::new(Mutex::new(GamepadState::default()));
}

/// Android KeyEvent button codes
pub mod keycodes {
    pub const BUTTON_A: i32 = 96;      // X
    pub const BUTTON_B: i32 = 97;      // ○
    pub const BUTTON_X: i32 = 99;      // □
    pub const BUTTON_Y: i32 = 100;     // △
    pub const BUTTON_L1: i32 = 102;
    pub const BUTTON_R1: i32 = 103;
    pub const BUTTON_L2: i32 = 104;
    pub const BUTTON_R2: i32 = 105;
    pub const BUTTON_THUMBL: i32 = 106;
    pub const BUTTON_THUMBR: i32 = 107;
    pub const BUTTON_START: i32 = 108; // Options
    pub const BUTTON_SELECT: i32 = 109; // Create
    pub const BUTTON_MODE: i32 = 110;  // PS button
    pub const DPAD_UP: i32 = 19;
    pub const DPAD_DOWN: i32 = 20;
    pub const DPAD_LEFT: i32 = 21;
    pub const DPAD_RIGHT: i32 = 22;
}

/// Called from lib.rs when a gamepad button event is received
pub fn handle_button(key_code: i32, pressed: bool) {
    let mut state = GAMEPAD_STATE.lock().unwrap();
    
    match key_code {
        keycodes::BUTTON_A => state.btn_south = pressed,
        keycodes::BUTTON_B => state.btn_east = pressed,
        keycodes::BUTTON_X => state.btn_west = pressed,
        keycodes::BUTTON_Y => state.btn_north = pressed,
        keycodes::BUTTON_L1 => state.btn_l1 = pressed,
        keycodes::BUTTON_R1 => state.btn_r1 = pressed,
        keycodes::BUTTON_L2 => state.btn_l2 = pressed,
        keycodes::BUTTON_R2 => state.btn_r2 = pressed,
        keycodes::BUTTON_THUMBL => state.btn_thumbl = pressed,
        keycodes::BUTTON_THUMBR => state.btn_thumbr = pressed,
        keycodes::BUTTON_START => state.btn_start = pressed,
        keycodes::BUTTON_SELECT => state.btn_select = pressed,
        keycodes::BUTTON_MODE => state.btn_mode = pressed,
        keycodes::DPAD_UP => state.btn_dpad_up = pressed,
        keycodes::DPAD_DOWN => state.btn_dpad_down = pressed,
        keycodes::DPAD_LEFT => state.btn_dpad_left = pressed,
        keycodes::DPAD_RIGHT => state.btn_dpad_right = pressed,
        _ => {}
    }
}

/// Called from lib.rs when stick/trigger motion is received (future)
pub fn handle_axis(left_x: f32, left_y: f32, right_x: f32, right_y: f32, l2: f32, r2: f32) {
    let mut state = GAMEPAD_STATE.lock().unwrap();
    state.left_stick_x = left_x;
    state.left_stick_y = left_y;
    state.right_stick_x = right_x;
    state.right_stick_y = right_y;
    state.l2_trigger = l2;
    state.r2_trigger = r2;
}

/// Get raw gamepad state
pub fn get_state() -> GamepadState {
    GAMEPAD_STATE.lock().unwrap().clone()
}

/// Get high-level actions (one-shot, fires on button DOWN edge)
/// Call this once per frame to get triggered actions
pub fn poll_actions() -> GamepadActions {
    let current = GAMEPAD_STATE.lock().unwrap().clone();
    let mut prev = PREV_STATE.lock().unwrap();
    
    // Detect rising edges (button just pressed)
    let actions = GamepadActions {
        // Media
        play_pause: current.btn_south && !prev.btn_south,      // X
        seek_back: current.btn_l1 && !prev.btn_l1,             // L1
        seek_forward: current.btn_r1 && !prev.btn_r1,          // R1
        
        // UI
        toggle_ui: current.btn_north && !prev.btn_north,       // △
        confirm: current.btn_west && !prev.btn_west,           // □
        back: current.btn_east && !prev.btn_east,              // ○
        
        // VR
        reset_view: current.btn_thumbl && !prev.btn_thumbl,    // L3
        toggle_vr_mode: current.btn_thumbr && !prev.btn_thumbr, // R3
        
        // App
        open_settings: current.btn_start && !prev.btn_start,   // Options
        open_file_picker: current.btn_select && !prev.btn_select, // Create
        exit_app: current.btn_mode && !prev.btn_mode,          // PS
        
        // Zoom (continuous while held)
        zoom_in: current.btn_r2,
        zoom_out: current.btn_l2,
        
        // Navigation
        nav_up: current.btn_dpad_up && !prev.btn_dpad_up,
        nav_down: current.btn_dpad_down && !prev.btn_dpad_down,
    };
    
    // Update previous state
    *prev = current;
    
    actions
}

/// Dummy struct for API compatibility
pub struct GamepadReader;

impl GamepadReader {
    pub fn new() -> Self {
        info!("GamepadReader: Modular winit-based input initialized");
        Self
    }
    
    pub fn get_state(&self) -> GamepadState {
        get_state()
    }
    
    pub fn poll_actions(&self) -> GamepadActions {
        poll_actions()
    }
}

// HAT axis state for D-pad (received from JNI)
lazy_static! {
    static ref HAT_STATE: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0)); // (hat_x, hat_y)
}

/// Get current D-pad HAT state
pub fn get_hat_state() -> (f32, f32) {
    *HAT_STATE.lock().unwrap()
}

// JNI Export: Receive gamepad button from Java
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onGamepadButton(
    _env: jni::JNIEnv,
    _class: jni::objects::JObject,
    button_code: jni::sys::jint,
    pressed: jni::sys::jboolean,
) {
    let pressed = pressed != 0;
    handle_button(button_code, pressed);
}

// JNI Export: Receive gamepad axis from Java (includes HAT_X/HAT_Y for D-pad)
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onGamepadAxis(
    _env: jni::JNIEnv,
    _class: jni::objects::JObject,
    left_x: jni::sys::jfloat,
    left_y: jni::sys::jfloat,
    right_x: jni::sys::jfloat,
    right_y: jni::sys::jfloat,
    l2: jni::sys::jfloat,
    r2: jni::sys::jfloat,
) {
    // Store axis state if needed
    if let Ok(mut state) = GAMEPAD_STATE.lock() {
        state.left_stick_x = left_x;
        state.left_stick_y = left_y;
        state.right_stick_x = right_x;
        state.right_stick_y = right_y;
        state.l2_trigger = l2;
        state.r2_trigger = r2;
    }
}

// JNI Export: Receive HAT axis (D-pad) from Java - separate callback for clarity
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onDpadAxis(
    _env: jni::JNIEnv,
    _class: jni::objects::JObject,
    hat_x: jni::sys::jfloat,
    hat_y: jni::sys::jfloat,
) {
    if let Ok(mut hat) = HAT_STATE.lock() {
        *hat = (hat_x, hat_y);
    }
    info!("JNI: D-pad HAT x={} y={}", hat_x, hat_y);
}
