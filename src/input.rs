//! Input handling module
//!
//! Handles PS5 DualSense controller, keyboard, and mouse input.

use gilrs::{Gilrs, Button, Event, EventType};
use glam::Vec2;
use log::info;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};


/// Controller state for cursor/interaction
pub struct InputState {
    gilrs: Option<Gilrs>,
    pub cursor_position: Vec2,
    pub left_stick: Vec2,
    pub right_stick: Vec2,
    pub primary_action: bool,   // Cross / Left Click
    pub secondary_action: bool, // Circle / Right Click
    pub menu_action: bool,      // Options / Start / Menu
    pub play_pause: bool,
    pub seek_forward: bool,
    pub seek_backward: bool,
    pub zoom: bool,
}

impl InputState {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => {
                info!("Gamepad system initialized");
                Some(g)
            }
            Err(e) => {
                info!("No gamepad support: {:?}", e);
                None
            }
        };
        
        Self {
            gilrs,
            cursor_position: Vec2::ZERO,
            left_stick: Vec2::ZERO,
            right_stick: Vec2::ZERO,
            primary_action: false,
            secondary_action: false,
            menu_action: false,
            play_pause: false,
            seek_forward: false,
            seek_backward: false,
            zoom: false,
        }
    }
    
    /// Poll for gamepad events
    pub fn update(&mut self) {
        // Collect events first to avoid borrow checker issues
        let events: Vec<_> = if let Some(gilrs) = &mut self.gilrs {
            let mut events = Vec::new();
            while let Some(Event { id: _, event, .. }) = gilrs.next_event() {
                events.push(event);
            }
            events
        } else {
            Vec::new()
        };
        
        // Process collected events
        for event in events {
            info!("RAW INPUT: {:?}", event);
            match event {
                EventType::ButtonPressed(button, _) => {
                    self.handle_button(button, true);
                }
                EventType::ButtonReleased(button, _) => {
                    self.handle_button(button, false);
                }
                EventType::AxisChanged(axis, value, _) => {
                    self.handle_axis(axis, value);
                }
                _ => {}
            }
        }
        
        // Move cursor based on right stick
        self.cursor_position += self.right_stick * 0.02;
        self.cursor_position = self.cursor_position.clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
    }
    
    fn handle_button(&mut self, button: Button, pressed: bool) {
        match button {
            Button::South => self.play_pause = pressed,       // X -> Play/Pause
            Button::West => self.primary_action = pressed,    // Square -> Click
            Button::East => self.secondary_action = pressed,  // Circle -> Back
            Button::Start => self.menu_action = pressed,      // Options -> Menu UI
            Button::RightTrigger2 => self.seek_forward = pressed, // R2 -> Seek Forward
            Button::LeftTrigger2 => self.seek_backward = pressed, // L2 -> Seek Backward
            Button::RightTrigger => self.zoom = pressed,      // R1 -> Zoom
            _ => {}
        }
    }
    
    fn handle_axis(&mut self, axis: gilrs::Axis, value: f32) {
        // Apply deadzone
        let value = if value.abs() < 0.1 { 0.0 } else { value };
        
        match axis {
            gilrs::Axis::LeftStickX => self.left_stick.x = value,
            gilrs::Axis::LeftStickY => self.left_stick.y = -value, // Inverted
            gilrs::Axis::RightStickX => self.right_stick.x = value,
            gilrs::Axis::RightStickY => self.right_stick.y = -value,
            _ => {}
        }
    }

    pub fn handle_keyboard_input(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        match &event.logical_key {
            Key::Named(NamedKey::Space) |
            Key::Named(NamedKey::Enter) => self.play_pause = pressed,
            
            Key::Named(NamedKey::Escape) => self.menu_action = pressed,
            
            Key::Named(NamedKey::ArrowRight) => self.seek_forward = pressed,
            Key::Named(NamedKey::ArrowLeft) => self.seek_backward = pressed,
            
             _ => {}
        }
    }
}
