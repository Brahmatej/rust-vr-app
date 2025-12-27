//! Sensor module for Android gyroscope access via NDK
//!
//! Uses ndk-sys FFI bindings to access the Game Rotation Vector sensor,
//! which is ideal for VR head tracking (no magnetic interference).

use glam::Quat;
use log::info;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Sensor type constants
const ASENSOR_TYPE_GAME_ROTATION_VECTOR: i32 = 15;
const ASENSOR_TYPE_GYROSCOPE: i32 = 4;

/// Manages sensor input for head tracking
pub struct SensorInput {
    sensor_manager: *mut ndk_sys::ASensorManager,
    event_queue: *mut ndk_sys::ASensorEventQueue,
    looper: *mut ndk_sys::ALooper,
    
    // Current orientation from sensors
    pub orientation: Quat,
    
    // Gyroscope data for integration
    gyro_x: f32,
    gyro_y: f32,
    gyro_z: f32,
    
    // Accumulated rotation from gyroscope
    pitch: f32,
    yaw: f32,
    roll: f32,
    
    initialized: bool,
}

// Safety: sensor pointers are used only from main thread
unsafe impl Send for SensorInput {}
unsafe impl Sync for SensorInput {}

impl SensorInput {
    pub fn new() -> Self {
        let mut input = Self {
            sensor_manager: ptr::null_mut(),
            event_queue: ptr::null_mut(),
            looper: ptr::null_mut(),
            orientation: Quat::IDENTITY,
            gyro_x: 0.0,
            gyro_y: 0.0,
            gyro_z: 0.0,
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            initialized: false,
        };
        
        input.init_sensors();
        input
    }
    
    fn init_sensors(&mut self) {
        info!("Initializing gyroscope sensors...");
        
        unsafe {
            // Get the sensor manager instance for this package
            self.sensor_manager = ndk_sys::ASensorManager_getInstanceForPackage(
                b"com.vrapp.core\0".as_ptr()
            );
            
            if self.sensor_manager.is_null() {
                info!("Failed to get ASensorManager, trying fallback");
                // Fallback to deprecated getInstance
                self.sensor_manager = ndk_sys::ASensorManager_getInstance();
            }
            
            if self.sensor_manager.is_null() {
                info!("ASensorManager not available");
                return;
            }
            info!("Got ASensorManager");
            
            // Try Game Rotation Vector first (best for VR)
            let mut sensor = ndk_sys::ASensorManager_getDefaultSensor(
                self.sensor_manager,
                ASENSOR_TYPE_GAME_ROTATION_VECTOR,
            );
            
            if sensor.is_null() {
                info!("Game Rotation Vector not available, trying gyroscope");
                sensor = ndk_sys::ASensorManager_getDefaultSensor(
                    self.sensor_manager,
                    ASENSOR_TYPE_GYROSCOPE,
                );
            }
            
            if sensor.is_null() {
                info!("No rotation sensors available");
                return;
            }
            info!("Got rotation sensor");
            
            // Get or create a looper for this thread
            self.looper = ndk_sys::ALooper_forThread();
            if self.looper.is_null() {
                self.looper = ndk_sys::ALooper_prepare(0);
            }
            
            if self.looper.is_null() {
                info!("Failed to get ALooper");
                return;
            }
            info!("Got ALooper");
            
            // Create the sensor event queue
            self.event_queue = ndk_sys::ASensorManager_createEventQueue(
                self.sensor_manager,
                self.looper,
                0, // ident
                None, // no callback
                ptr::null_mut(),
            );
            
            if self.event_queue.is_null() {
                info!("Failed to create sensor event queue");
                return;
            }
            info!("Created sensor event queue");
            
            // Enable the sensor
            let result = ndk_sys::ASensorEventQueue_enableSensor(self.event_queue, sensor);
            if result < 0 {
                info!("Failed to enable sensor: {}", result);
                return;
            }
            
            // Set event rate to ~60Hz (16ms = 16000 microseconds)
            ndk_sys::ASensorEventQueue_setEventRate(self.event_queue, sensor, 16000);
            
            self.initialized = true;
            info!("Gyroscope sensors initialized successfully!");
        }
    }
    
    /// Poll sensor events and update orientation
    pub fn update(&mut self, dt: f32) {
        if !self.initialized || self.event_queue.is_null() {
            // Fallback: gentle simulated motion
            self.simulate_motion(dt);
            return;
        }
        
        unsafe {
            // Poll events without blocking
            let mut event: ndk_sys::ASensorEvent = std::mem::zeroed();
            
            // Get all pending events
            while ndk_sys::ASensorEventQueue_getEvents(self.event_queue, &mut event, 1) > 0 {
                match event.type_ {
                    15 => {
                        // Game Rotation Vector (quaternion directly!)
                        // data[0] = x, data[1] = y, data[2] = z, data[3] = w (optional)
                        let x = event.__bindgen_anon_1.__bindgen_anon_1.data[0];
                        let y = event.__bindgen_anon_1.__bindgen_anon_1.data[1];
                        let z = event.__bindgen_anon_1.__bindgen_anon_1.data[2];
                        // Compute w from unit quaternion constraint
                        let w = (1.0 - x*x - y*y - z*z).max(0.0).sqrt();
                        self.orientation = Quat::from_xyzw(x, y, z, w).normalize();
                    }
                    4 => {
                        // Gyroscope (rate of rotation in rad/s)
                        self.gyro_x = event.__bindgen_anon_1.__bindgen_anon_1.data[0];
                        self.gyro_y = event.__bindgen_anon_1.__bindgen_anon_1.data[1];
                        self.gyro_z = event.__bindgen_anon_1.__bindgen_anon_1.data[2];
                        
                        // Integrate gyroscope to get orientation
                        self.pitch += self.gyro_x * dt;
                        self.yaw += self.gyro_z * dt;
                        self.roll += self.gyro_y * dt;
                        
                        // Convert to quaternion
                        self.orientation = Quat::from_euler(
                            glam::EulerRot::YXZ,
                            self.yaw,
                            self.pitch,
                            self.roll,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    
    /// Fallback simulated motion when sensors unavailable
    fn simulate_motion(&mut self, dt: f32) {
        static mut TIME: f32 = 0.0;
        unsafe {
            TIME += dt;
            let breathing = (TIME * 0.5).sin() * 0.01;
            let sway = (TIME * 0.3).sin() * 0.005;
            self.orientation = Quat::from_euler(
                glam::EulerRot::YXZ,
                0.0,
                breathing,
                sway,
            );
        }
    }
    
    /// Reset orientation to identity
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.pitch = 0.0;
        self.yaw = 0.0;
        self.roll = 0.0;
        self.orientation = Quat::IDENTITY;
        info!("Orientation reset to identity");
    }
    
    /// Check if sensors are available
    pub fn is_available(&self) -> bool {
        self.initialized
    }
}

impl Default for SensorInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SensorInput {
    fn drop(&mut self) {
        unsafe {
            if !self.event_queue.is_null() && !self.sensor_manager.is_null() {
                ndk_sys::ASensorManager_destroyEventQueue(self.sensor_manager, self.event_queue);
            }
        }
    }
}
