//! Hardware-accelerated video thumbnail pipeline.
//!
//! Rust asks Java to extract a poster frame (`request`) — Java uses
//! MediaMetadataRetriever (hardware-backed) on a thread pool. The finished RGBA
//! frame comes back through the `onThumbnail` JNI callback, is queued here, and
//! the UI drains it (`drain`) to upload as a GPU texture.
//!
//! Each thumbnail also gets an average colour (for an ambient glow) computed with
//! a NEON SIMD reduction, runtime-detected, with a scalar fallback.

use android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::sys::jobject;
use log::error;
use std::sync::Mutex;

/// A finished thumbnail ready for the UI to upload as a texture.
pub struct ThumbResult {
    pub path: String,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub glow: [u8; 3],
}

/// Completed thumbnails waiting to be drained by the UI.
static DONE: Mutex<Vec<ThumbResult>> = Mutex::new(Vec::new());

/// Ask Java to generate a thumbnail for `path` at target `w`x`h`. Non-blocking;
/// the result arrives later via the `onThumbnail` JNI callback.
pub fn request(app: &AndroidApp, path: &str, w: i32, h: i32) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(e) => { error!("thumbs: attach failed: {:?}", e); return; }
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    let j_path = match env.new_string(path) {
        Ok(s) => s,
        Err(e) => { error!("thumbs: new_string failed: {:?}", e); return; }
    };
    if let Err(e) = env.call_method(
        &activity,
        "requestThumbnail",
        "(Ljava/lang/String;II)V",
        &[JValue::Object(&j_path.into()), JValue::Int(w), JValue::Int(h)],
    ) {
        error!("thumbs: requestThumbnail call failed: {:?}", e);
    }
}

/// Take all finished thumbnails (clears the queue).
pub fn drain() -> Vec<ThumbResult> {
    if let Ok(mut done) = DONE.lock() {
        std::mem::take(&mut *done)
    } else {
        Vec::new()
    }
}

// ── JNI callback from Java ──────────────────────────────────────────────────────

/// Java calls this with a decoded RGBA thumbnail (or w==0/h==0 on failure).
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onThumbnail(
    mut env: jni::JNIEnv,
    _class: JObject,
    path: jni::objects::JString,
    width: jni::sys::jint,
    height: jni::sys::jint,
    rgba: jni::objects::JByteArray,
) {
    let path: String = match env.get_string(&path) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    if width <= 0 || height <= 0 {
        return; // extraction failed on the Java side
    }

    let len = env.get_array_length(&rgba).unwrap_or(0) as usize;
    if len == 0 {
        return;
    }

    let mut buf = vec![0u8; len];
    {
        // get_byte_array_region wants &mut [i8]; reinterpret to avoid a copy.
        let dst: &mut [i8] = std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut i8, len);
        if env.get_byte_array_region(&rgba, 0, dst).is_err() {
            return;
        }
    }

    let glow = average_rgb(&buf);

    if let Ok(mut done) = DONE.lock() {
        done.push(ThumbResult {
            path,
            w: width as u32,
            h: height as u32,
            rgba: buf,
            glow,
        });
    }
}

// ── Average colour (NEON SIMD + scalar fallback) ────────────────────────────────

/// Mean RGB over an RGBA8 buffer, used for the card's ambient glow.
fn average_rgb(rgba: &[u8]) -> [u8; 3] {
    let px = (rgba.len() / 4) as u64;
    if px == 0 {
        return [40, 40, 48];
    }

    let (r, g, b);
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            let (rr, gg, bb) = unsafe { sum_rgb_neon(rgba) };
            r = rr; g = gg; b = bb;
        } else {
            let (rr, gg, bb) = sum_rgb_scalar(rgba);
            r = rr; g = gg; b = bb;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let (rr, gg, bb) = sum_rgb_scalar(rgba);
        r = rr; g = gg; b = bb;
    }

    [(r / px) as u8, (g / px) as u8, (b / px) as u8]
}

fn sum_rgb_scalar(rgba: &[u8]) -> (u64, u64, u64) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for px in rgba.chunks_exact(4) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
    }
    (r, g, b)
}

/// NEON reduction: de-interleave 16 RGBA pixels per iteration with vld4q_u8, then
/// widen-accumulate each channel. Falls back to scalar for the tail.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn sum_rgb_neon(rgba: &[u8]) -> (u64, u64, u64) {
    use std::arch::aarch64::*;

    let mut acc_r = vdupq_n_u32(0);
    let mut acc_g = vdupq_n_u32(0);
    let mut acc_b = vdupq_n_u32(0);

    let chunks = rgba.len() / 64; // 16 pixels * 4 bytes
    let mut ptr = rgba.as_ptr();
    for _ in 0..chunks {
        let v = vld4q_u8(ptr); // v.0=R, v.1=G, v.2=B, v.3=A (16 lanes each)
        // widen 8->16 pairwise-add, then 16->32 pairwise-accumulate
        acc_r = vpadalq_u16(acc_r, vpaddlq_u8(v.0));
        acc_g = vpadalq_u16(acc_g, vpaddlq_u8(v.1));
        acc_b = vpadalq_u16(acc_b, vpaddlq_u8(v.2));
        ptr = ptr.add(64);
    }

    let mut r = (vgetq_lane_u32(acc_r, 0)
        + vgetq_lane_u32(acc_r, 1)
        + vgetq_lane_u32(acc_r, 2)
        + vgetq_lane_u32(acc_r, 3)) as u64;
    let mut g = (vgetq_lane_u32(acc_g, 0)
        + vgetq_lane_u32(acc_g, 1)
        + vgetq_lane_u32(acc_g, 2)
        + vgetq_lane_u32(acc_g, 3)) as u64;
    let mut b = (vgetq_lane_u32(acc_b, 0)
        + vgetq_lane_u32(acc_b, 1)
        + vgetq_lane_u32(acc_b, 2)
        + vgetq_lane_u32(acc_b, 3)) as u64;

    // scalar tail for remaining pixels
    let tail = &rgba[chunks * 64..];
    let (tr, tg, tb) = sum_rgb_scalar(tail);
    r += tr; g += tg; b += tb;

    (r, g, b)
}
