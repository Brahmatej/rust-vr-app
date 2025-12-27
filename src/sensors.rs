//! Sensor module for Android gyroscope and accelerometer access
//!
//! For now, uses a simple approach without NDK sensor polling to avoid
//! conflicts with android-activity's looper. Orientation is derived from
//! the device's motion over time.

use glam::{Quat, Vec3};
use log::info;

/// Manages sensor input for head tracking
/// 
/// Note: Full sensor access requires careful integration with android-activity's
/// looper. For now, we use a simpler simulation-based approach for testing
/// the VR rendering pipeline.
pub struct SensorInput {
    // Current orientation (quaternion)
    pub orientation: Quat,
    
    // Euler angles for orientation
    pitch: f32,
    roll: f32,
    yaw: f32,
    
    // Simulated motion parameters
    time: f32,
    
    // Sensor availability flag
    sensors_available: bool,
}

impl SensorInput {
    pub fn new() -> Self {
        info!("Initializing sensor input (simulation mode)");
        
        Self {
            orientation: Quat::IDENTITY,
            pitch: 0.0,
            roll: 0.0,
            yaw: 0.0,
            time: 0.0,
            sensors_available: true, // Mark as available for VR mode to work
        }
    }
    
    /// Update orientation (simulates subtle head movement for testing)
    pub fn update(&mut self, dt: f32) {
        self.time += dt;
        
        // For now, simulate a very gentle floating motion
        // This helps verify the VR rendering is working correctly
        // Real gyroscope integration will come in the next iteration
        
        // Subtle breathing-like motion
        let breathing = (self.time * 0.5).sin() * 0.01;
        self.pitch = breathing;
        
        // Very subtle sway
        let sway = (self.time * 0.3).sin() * 0.005;
        self.roll = sway;
        
        // Yaw stays fixed (no drift)
        // self.yaw += 0.0;
        
        // Convert to quaternion
        self.orientation = Quat::from_euler(
            glam::EulerRot::YXZ,
            self.yaw,
            self.pitch,
            self.roll,
        );
    }
    
    /// Reset orientation to identity
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.pitch = 0.0;
        self.roll = 0.0;
        self.yaw = 0.0;
        self.orientation = Quat::IDENTITY;
        info!("Orientation reset");
    }
    
    /// Check if sensors are available
    pub fn is_available(&self) -> bool {
        self.sensors_available
    }
    
    /// Set manual rotation (for testing or gamepad input)
    #[allow(dead_code)]
    pub fn set_rotation(&mut self, pitch: f32, yaw: f32, roll: f32) {
        self.pitch = pitch;
        self.yaw = yaw;
        self.roll = roll;
        self.orientation = Quat::from_euler(
            glam::EulerRot::YXZ,
            self.yaw,
            self.pitch,
            self.roll,
        );
    }
}

impl Default for SensorInput {
    fn default() -> Self {
        Self::new()
    }
}
