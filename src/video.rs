use log::{info, error};
use jni::objects::{JObject, JValue};
use jni::sys::jobject;
use android_activity::AndroidApp;

pub struct VideoManager;

impl VideoManager {
    /// Launches the Android system file picker via MainActivity.launchVideoPicker()
    pub fn pick_video(app: &AndroidApp) {
        info!("VideoManager: Calling Java launchVideoPicker...");
        
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
        let mut env = vm.attach_current_thread().unwrap();
        
        // Get Activity
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
        
        // Call method: public void launchVideoPicker()
        match env.call_method(&activity, "launchVideoPicker", "()V", &[]) {
            Ok(_) => info!("VideoManager: Java method called successfully."),
            Err(e) => error!("VideoManager: Failed to call launchVideoPicker: {:?}", e),
        }
    }

}

// JNI Export to receive result
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onVideoPicked(
    mut env: jni::JNIEnv,
    _class: JObject,
    uri_jstring: jni::objects::JString,
) {
    // Convert Java String to Rust String
    let uri: String = env.get_string(&uri_jstring)
        .expect("Couldn't get java string!")
        .into();
        
    info!("JNI Native: Video Picked URI = {}", uri);
}

use std::sync::atomic::{AtomicI32, Ordering};

/// Pending video file descriptor from Java (set by onVideoFdReady)
pub static PENDING_VIDEO_FD: AtomicI32 = AtomicI32::new(-1);

/// Check if there's a pending video fd
pub fn get_pending_fd() -> Option<i32> {
    let fd = PENDING_VIDEO_FD.swap(-1, Ordering::SeqCst);
    if fd >= 0 { Some(fd) } else { None }
}

// JNI Export to receive file descriptor for NDK decoder
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onVideoFdReady(
    _env: jni::JNIEnv,
    _class: JObject,
    fd: jni::sys::jint,
) {
    info!("JNI Native: Got video fd = {}", fd);
    PENDING_VIDEO_FD.store(fd, Ordering::SeqCst);
}

// ── Recovery baseline stub ──────────────────────────────────────────────────────
// Surface::ROTATION_0/90/180/270 as reported by MainActivity.reportDisplayRotation().
// Head tracking needs this: the sensor quaternion is expressed against the device's
// PORTRAIT-referenced frame, so in landscape the screen is rotated relative to it and
// the tracking axes must be rotated back by the same amount (see sensors.rs).
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onDisplayRotation(
    _env: jni::JNIEnv,
    _class: JObject,
    rotation: jni::sys::jint,
) {
    crate::sensors::set_display_rotation(rotation as i32);
}

/// Start audio from file path (for file browser selections)
pub fn start_audio_from_path(app: &AndroidApp, path: &str) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    let path_jstr = env.new_string(path).unwrap();
    
    match env.call_method(&activity, "startAudioFromPath", "(Ljava/lang/String;)V", &[JValue::Object(&path_jstr.into())]) {
        Ok(_) => info!("Audio started from path: {}", path),
        Err(e) => error!("Failed to start audio: {:?}", e),
    }
}

/// Pause Java MediaPlayer audio
pub fn pause_audio(app: &AndroidApp) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "pauseAudio", "()V", &[]) {
        Ok(_) => info!("Audio paused"),
        Err(e) => error!("Failed to pause audio: {:?}", e),
    }
}

/// Resume Java MediaPlayer audio
pub fn resume_audio(app: &AndroidApp) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "resumeAudio", "()V", &[]) {
        Ok(_) => info!("Audio resumed"),
        Err(e) => error!("Failed to resume audio: {:?}", e),
    }
}

/// Seek Java MediaPlayer audio to position (milliseconds)
pub fn seek_audio(app: &AndroidApp, position_ms: i32) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "seekAudio", "(I)V", &[JValue::Int(position_ms)]) {
        Ok(_) => info!("Audio seek to {}ms", position_ms),
        Err(e) => error!("Failed to seek audio: {:?}", e),
    }
}

/// Current audio position in milliseconds, or -1 when nothing is playing.
///
/// This is the A/V master clock: the video decoder paces itself against it
/// instead of against wall-clock time, which is what keeps the picture locked
/// to the sound across pause, resume and seek.
pub fn audio_position_ms(app: &AndroidApp) -> i32 {
    let vm = match unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) } {
        Ok(vm) => vm,
        Err(_) => return -1,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(env) => env,
        Err(_) => return -1,
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };

    match env.call_method(&activity, "getAudioPositionMs", "()I", &[]) {
        Ok(v) => v.i().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Increase system media volume
pub fn volume_up(app: &AndroidApp) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "volumeUp", "()V", &[]) {
        Ok(_) => info!("Volume up"),
        Err(e) => error!("Failed to increase volume: {:?}", e),
    }
}

/// Decrease system media volume
pub fn volume_down(app: &AndroidApp) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "volumeDown", "()V", &[]) {
        Ok(_) => info!("Volume down"),
        Err(e) => error!("Failed to decrease volume: {:?}", e),
    }
}

/// Check D-pad volume buttons (called from game loop with HAT values)
#[allow(dead_code)]
pub fn check_volume_buttons(app: &AndroidApp, left: bool, right: bool) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    
    match env.call_method(&activity, "checkVolumeButtons", "(ZZ)V", &[
        JValue::Bool(left as u8),
        JValue::Bool(right as u8),
    ]) {
        Ok(_) => {},
        Err(e) => error!("Failed to check volume buttons: {:?}", e),
    }
}
