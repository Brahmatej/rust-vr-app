use log::{info, error};
use jni::objects::{JObject, JValue};
use jni::sys::jobject;
use android_activity::AndroidApp;

/// Video frame data received from Java
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

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

    /// Fetches the latest video frame from Java
    pub fn get_video_frame(app: &AndroidApp) -> Option<VideoFrame> {
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
        let mut env = vm.attach_current_thread().unwrap();
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };

        // Get dimensions
        let width_result = env.call_method(&activity, "getVideoWidth", "()I", &[]);
        let width = match width_result {
            Ok(val) => val.i().unwrap_or(0) as u32,
            Err(_) => return None,
        };
        
        let height_result = env.call_method(&activity, "getVideoHeight", "()I", &[]);
        let height = match height_result {
            Ok(val) => val.i().unwrap_or(0) as u32,
            Err(_) => return None,
        };

        // Get frame bytes
        let frame_result = env.call_method(&activity, "getVideoFrame", "()[B", &[]);
        match frame_result {
            Ok(val) => {
                let obj = match val.l() {
                    Ok(o) => o,
                    Err(_) => return None,
                };
                if obj.is_null() {
                    return None;
                }
                let byte_array: jni::objects::JByteArray = obj.into();
                let len = env.get_array_length(&byte_array).unwrap_or(0) as usize;
                if len == 0 {
                    return None;
                }
                let mut buffer = vec![0i8; len];
                if env.get_byte_array_region(&byte_array, 0, &mut buffer).is_err() {
                    return None;
                }
                // Convert i8 to u8
                let data: Vec<u8> = buffer.iter().map(|&b| b as u8).collect();
                Some(VideoFrame { data, width, height })
            }
            Err(_) => None,
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
