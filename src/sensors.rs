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

/// Current Surface.ROTATION_* of the display, pushed in from Java via
/// MainActivity.onDisplayRotation -> video.rs. Defaults to ROTATION_90 (the
/// usual landscape the headset sits in) until the first report arrives.
static DISPLAY_ROTATION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

pub fn set_display_rotation(rotation: i32) {
    info!("Sensors: display rotation = {}", rotation);
    DISPLAY_ROTATION.store(rotation, std::sync::atomic::Ordering::Relaxed);
}

/// Convert an Android rotation-vector quaternion into our render space.
///
/// Android reports device->world in **ENU**: world X=east, Y=north, Z=UP, and the
/// device frame is referenced to PORTRAIT (X=right, Y=up the screen, Z=out of it).
/// Our renderer is OpenGL-style: Y=up, -Z=forward, and it builds the view matrix as
/// `Mat4::from_quat(orientation.inverse())`, so `orientation` must be a genuine
/// camera->world rotation in that space.
///
/// Blind per-axis sign flips can't express this: it's a change of basis on BOTH
/// sides. The world side needs ENU(Z-up) -> GL(Y-up), i.e. a -90 deg rotation about
/// X. The device side needs the screen rotation undone, i.e. a rotation about the
/// device's Z by the current display rotation. Composing those is what makes every
/// axis (yaw/pitch/roll) come out in the right direction simultaneously.
fn sensor_quat_to_render(x: f32, y: f32, z: f32, w: f32) -> Quat {
    use std::f32::consts::FRAC_PI_2;

    let q_sensor = Quat::from_xyzw(x, y, z, w);

    // ENU (Z-up) -> GL (Y-up)
    let world_fix = Quat::from_rotation_x(-FRAC_PI_2);

    // Undo the portrait->landscape screen rotation on the device side.
    let screen_angle = match DISPLAY_ROTATION.load(std::sync::atomic::Ordering::Relaxed) {
        1 => FRAC_PI_2,             // ROTATION_90
        2 => std::f32::consts::PI,  // ROTATION_180
        3 => -FRAC_PI_2,            // ROTATION_270
        _ => 0.0,                   // ROTATION_0
    };
    let device_fix = Quat::from_rotation_z(screen_angle);

    (world_fix * q_sensor * device_fix).normalize()
}

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
                            
                            new_quat = sensor_quat_to_render(x, y, z, w);
                            updated = true;
                        
                        } else if sensor_type == ASENSOR_TYPE_GYROSCOPE {
                            let gx = event.__bindgen_anon_1.__bindgen_anon_1.data[0];
                            let gy = event.__bindgen_anon_1.__bindgen_anon_1.data[1];
                            let gz = event.__bindgen_anon_1.__bindgen_anon_1.data[2];
                            let ts = event.timestamp;
                            
                            if last_ts > 0 {
                                let dt = (ts - last_ts) as f32 / 1_000_000_000.0;
                                if dt < 0.2 {
                                    // Integrate in the DEVICE frame (the frame the gyro
                                    // actually reports in), then run the result through the
                                    // same ENU->GL + screen-rotation change of basis as the
                                    // rotation-vector path so both paths agree.
                                    gyro_pitch += gx * dt;
                                    gyro_yaw   += gy * dt;
                                    gyro_roll  += gz * dt;

                                    let device_q = Quat::from_euler(
                                        glam::EulerRot::YXZ,
                                        gyro_yaw,
                                        gyro_pitch,
                                        gyro_roll,
                                    );
                                    let (dx, dy, dz, dw) =
                                        (device_q.x, device_q.y, device_q.z, device_q.w);
                                    new_quat = sensor_quat_to_render(dx, dy, dz, dw);
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
