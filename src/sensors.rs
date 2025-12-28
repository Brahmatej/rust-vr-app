//! Sensor module for Android gyroscope access via NDK
//!
//! Uses DEDICATED THREAD with LOOPER.
//! Includes aggressive logging to diagnose why events were missing.

use glam::Quat;
use log::{info, error, warn};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

// Sensor type constants
const ASENSOR_TYPE_GAME_ROTATION_VECTOR: i32 = 15;
const ASENSOR_TYPE_ROTATION_VECTOR: i32 = 11;
const ASENSOR_TYPE_GYROSCOPE: i32 = 4;

// Static storage for reference orientation (survives activity recreation)
static SAVED_REFERENCE: OnceLock<Mutex<Quat>> = OnceLock::new();

/// Thread-safe shared state for orientation
struct SharedState {
    orientation: Quat,        // Current raw orientation from sensor
    reference: Quat,          // Reference orientation (Tare)
    running: bool,
}

/// Manages sensor input for VR head tracking
pub struct SensorInput {
    state: Arc<Mutex<SharedState>>,
    _thread_handle: Option<thread::JoinHandle<()>>,
}

unsafe impl Send for SensorInput {}
unsafe impl Sync for SensorInput {}

impl SensorInput {
    pub fn new() -> Self {
        // Load saved reference orientation if available
        let saved_ref = SAVED_REFERENCE
            .get_or_init(|| Mutex::new(Quat::IDENTITY))
            .lock()
            .map(|g| *g)
            .unwrap_or(Quat::IDENTITY);
        
        info!("SensorInput: Using saved reference: {:?}", saved_ref);
        
        let state = Arc::new(Mutex::new(SharedState {
            orientation: Quat::IDENTITY,
            reference: saved_ref,  // Use saved reference
            running: true,
        }));
        
        let thread_state = state.clone();
        
        // Spawn dedicated sensor thread
        let handle = thread::spawn(move || {
            Self::sensor_loop(thread_state);
        });
        
        Self {
            state,
            _thread_handle: Some(handle),
        }
    }
    
    fn sensor_loop(state: Arc<Mutex<SharedState>>) {
        info!("THREAD: Sensor thread (LOOPER MODE) started");
        
        unsafe {
            // 1. Prepare Looper - CRITICAL FIX
            // We must pass ALOOPER_PREPARE_ALLOW_NON_CALLBACKS (1) to handle FDs without callbacks!
            let looper = ndk_sys::ALooper_prepare(ndk_sys::ALOOPER_PREPARE_ALLOW_NON_CALLBACKS as i32);
            if looper.is_null() {
                error!("THREAD: Failed to prepare ALOOPER");
                return;
            }
            info!("THREAD: Looper prepared correctly");
            
            // 2. Get Manager
            let mut pt = b"com.vrapp.core\0".as_ptr();
            let mut manager = ndk_sys::ASensorManager_getInstanceForPackage(pt);
            if manager.is_null() {
                manager = ndk_sys::ASensorManager_getInstance();
            }
            if manager.is_null() {
                 error!("THREAD: Failed to get Manager");
                 return;
            }
            
            // 3. Find Sensor - Prefer Rotation Vector (Type 11) for best compatibility
            let mut sensor = ndk_sys::ASensorManager_getDefaultSensor(
                manager, 
                ASENSOR_TYPE_ROTATION_VECTOR
            );
            let mut sensor_type = ASENSOR_TYPE_ROTATION_VECTOR;
            
            if sensor.is_null() {
                sensor = ndk_sys::ASensorManager_getDefaultSensor(
                    manager, 
                    ASENSOR_TYPE_GAME_ROTATION_VECTOR
                );
                sensor_type = ASENSOR_TYPE_GAME_ROTATION_VECTOR;
            }
            
            if sensor.is_null() {
                sensor = ndk_sys::ASensorManager_getDefaultSensor(
                    manager, 
                    ASENSOR_TYPE_GYROSCOPE
                );
                sensor_type = ASENSOR_TYPE_GYROSCOPE;
            }
            
            if sensor.is_null() {
                error!("THREAD: No sensor found");
                return;
            }
            info!("THREAD: Found sensor type: {}", sensor_type);
            
            // 4. Create Queue attached to Looper
            let ident = 17; // Random ident
            let queue = ndk_sys::ASensorManager_createEventQueue(
                manager,
                looper,
                ident,
                None,
                ptr::null_mut(),
            );
            
            if queue.is_null() {
                error!("THREAD: Failed to create Queue");
                return;
            }
            info!("THREAD: Queue created");
            
            // 5. Enable Sensor
            let status = ndk_sys::ASensorEventQueue_enableSensor(queue, sensor);
            if status < 0 {
                error!("THREAD: Enable failed: {}", status);
                return;
            }
            
            // Set rate (20ms) - safer rate
            ndk_sys::ASensorEventQueue_setEventRate(queue, sensor, 20000);
            info!("THREAD: Sensor enabled at 20ms rate");
            
            // 6. Loop
            let mut event: ndk_sys::ASensorEvent = std::mem::zeroed();
            let mut loop_count = 0;
            
            // Gyro integration
            let mut gyro_pitch = 0.0f32;
            let mut gyro_yaw = 0.0f32;
            let mut gyro_roll = 0.0f32;
            let mut last_ts = 0i64;
            
            while state.lock().unwrap().running {
                loop_count += 1;
                
                // Poll Looper
                let poll_id = ndk_sys::ALooper_pollAll(
                    100, 
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut()
                );
                
                if poll_id == ndk_sys::ALOOPER_POLL_TIMEOUT {
                    continue;
                }
                
                if poll_id == ndk_sys::ALOOPER_POLL_ERROR {
                    error!("THREAD: Poll Error");
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                
                if poll_id == ident {
                    // Data available!
                    let count = ndk_sys::ASensorEventQueue_getEvents(queue, &mut event, 1);
                    if count > 0 {
                        let mut new_quat = Quat::IDENTITY;
                        let mut updated = false;
                        
                        // Process
                         if sensor_type == ASENSOR_TYPE_GAME_ROTATION_VECTOR || sensor_type == ASENSOR_TYPE_ROTATION_VECTOR {
                            let x = event.__bindgen_anon_1.__bindgen_anon_1.data[0];
                            let y = event.__bindgen_anon_1.__bindgen_anon_1.data[1];
                            let z = event.__bindgen_anon_1.__bindgen_anon_1.data[2];
                            let w = event.__bindgen_anon_1.__bindgen_anon_1.data[3];
                            // Debug raw values
                            if loop_count % 30 == 0 {
                                // info!("DATA: {:.3} {:.3} {:.3} {:.3}", x, y, z, w);
                            }
                            
                            // Use (-y, x) mapping to fix cross-talk
                            // Previous attempts: (x,y) -> cross-talk. (y,-x) -> cross-talk.
                            // Trying (-y, x) which is the other 90-degree rotation.
                            
                            new_quat = Quat::from_xyzw(-y, x, z, w).normalize();
                            updated = true;
                        
                        } else if sensor_type == ASENSOR_TYPE_GYROSCOPE {
                            let gx = event.__bindgen_anon_1.__bindgen_anon_1.data[0];
                            let gy = event.__bindgen_anon_1.__bindgen_anon_1.data[1];
                            let gz = event.__bindgen_anon_1.__bindgen_anon_1.data[2];
                            let ts = event.timestamp;
                            
                            if last_ts > 0 {
                                let dt = (ts - last_ts) as f32 / 1_000_000_000.0;
                                if dt < 0.2 {
                                    // Match (-y, x) mapping
                                    // Pitch (X) -> -SensY (-gy)
                                    // Yaw (Y)   -> SensX (gx)
                                    // Roll (Z)  -> SensZ (gz)
                                    
                                    gyro_pitch -= gy * dt;
                                    gyro_yaw += gx * dt;
                                    gyro_roll += gz * dt;
                                    
                                    new_quat = Quat::from_euler(
                                        glam::EulerRot::YXZ,
                                        gyro_yaw,
                                        gyro_pitch,
                                        gyro_roll,
                                    );
                                    updated = true;
                                }
                            }
                            last_ts = ts;
                        }
                        
                        if updated {
                            if let Ok(mut s) = state.lock() {
                                s.orientation = new_quat;
                            }
                        }
                    }
                }
            }
            
            // Clean
            ndk_sys::ASensorEventQueue_disableSensor(queue, sensor);
            ndk_sys::ASensorManager_destroyEventQueue(manager, queue);
        }
    }
    
    pub fn update(&mut self, _dt: f32) {}

    pub fn get_orientation(&self) -> Quat {
        if let Ok(s) = self.state.lock() {
            // Return: Reference^-1 * Raw
            s.reference.inverse() * s.orientation
        } else {
            Quat::IDENTITY
        }
    }
    
    /// Recenter the view (Tare)
    pub fn recenter(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.reference = s.orientation;
            
            // Save to static storage for persistence across activity recreation
            if let Some(saved) = SAVED_REFERENCE.get() {
                if let Ok(mut g) = saved.lock() {
                    *g = s.reference;
                }
            }
            
            info!("Sensor Recalibrated/Centered (saved)");
        }
    }

    pub fn is_available(&self) -> bool {
        self._thread_handle.is_some()
    }
    
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.recenter();
    }
}

impl Default for SensorInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SensorInput {
    fn drop(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.running = false;
        }
    }
}
