//! WebView / browser JNI bridge.
//!
//! Rust → Java: calls control methods on MainActivity (loadUrl, tap, scroll, tabs,
//! engine switch, voice search, …) which forward to the active browser engine
//! (Chromium WebView or Firefox/Gecko).
//!
//! Java → Rust: `onWebFrame` pushes captured RGBA page frames; `onVoiceResult` /
//! `onVoiceError` deliver PS5-mic voice-search results.

use android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::sys::jobject;
use log::{error, info};
use std::sync::Mutex;

// ── Java → Rust shared state ────────────────────────────────────────────────────

/// Latest captured browser frame: (width, height, RGBA bytes).
static WEB_FRAME: Mutex<Option<(u32, u32, Vec<u8>)>> = Mutex::new(None);

/// Latest voice-search transcript (consumed by lib.rs).
static VOICE_RESULT: Mutex<Option<String>> = Mutex::new(None);

/// Free-list of frame buffers so the capture hot path allocates nothing in steady
/// state. The producer (`onWebFrame`) takes one; the render loop returns it via
/// `recycle` once uploaded to the GPU.
static WEB_FREE: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Take a buffer of at least `len` bytes from the free-list, or allocate one.
fn take_buf(len: usize) -> Vec<u8> {
    if let Ok(mut free) = WEB_FREE.lock() {
        if let Some(mut b) = free.pop() {
            b.clear();
            b.resize(len, 0);
            return b;
        }
    }
    vec![0u8; len]
}

/// Return a consumed frame buffer to the free-list (capped so it can't grow forever).
pub fn recycle(mut buf: Vec<u8>) {
    if let Ok(mut free) = WEB_FREE.lock() {
        if free.len() < 4 {
            buf.clear();
            free.push(buf);
        }
    }
}

// ── Rust → Java helpers ─────────────────────────────────────────────────────────

/// Run `body` with a JNIEnv attached to the current thread and the MainActivity obj.
fn with_activity<F: FnOnce(&mut jni::JNIEnv, &JObject)>(app: &AndroidApp, f: F) {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(e) => { error!("webview: attach_current_thread failed: {:?}", e); return; }
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };
    f(&mut env, &activity);
}

/// Call a `void name()` method on MainActivity.
fn call_void(app: &AndroidApp, name: &str) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(activity, name, "()V", &[]) {
            error!("webview: {} failed: {:?}", name, e);
        }
    });
}

// ── Navigation / page control ───────────────────────────────────────────────────

/// Load a URL in the active browser engine.
pub fn load_url(app: &AndroidApp, url: &str) {
    with_activity(app, |env, activity| {
        let j = match env.new_string(url) { Ok(s) => s, Err(_) => return };
        if let Err(e) = env.call_method(
            activity, "webViewLoadUrl", "(Ljava/lang/String;)V",
            &[JValue::Object(&JObject::from(j))],
        ) {
            error!("webview: webViewLoadUrl failed: {:?}", e);
        } else {
            info!("webview: loading {}", url);
        }
    });
}

pub fn go_back(app: &AndroidApp)    { call_void(app, "webViewGoBack"); }
pub fn go_forward(app: &AndroidApp) { call_void(app, "webViewGoForward"); }
pub fn reload(app: &AndroidApp)     { call_void(app, "webViewReload"); }
pub fn backspace(app: &AndroidApp)  { call_void(app, "webViewBackspace"); }
pub fn submit_enter(app: &AndroidApp) { call_void(app, "webViewEnter"); }

/// Select the browser engine: 0 = Chromium WebView, 1 = Firefox/Gecko.
pub fn set_engine(app: &AndroidApp, engine: i32) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "setBrowserEngine", "(I)V", &[JValue::Int(engine)],
        ) {
            error!("webview: setBrowserEngine failed: {:?}", e);
        }
    });
}

/// Resize the browser render surface (e.g. portrait for reels).
pub fn resize(app: &AndroidApp, width: i32, height: i32) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "webViewResize", "(II)V",
            &[JValue::Int(width), JValue::Int(height)],
        ) {
            error!("webview: webViewResize failed: {:?}", e);
        }
    });
}

/// Tap at a normalized (0..1) position on the page.
pub fn tap(app: &AndroidApp, x: f32, y: f32) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "webViewTap", "(FF)V",
            &[JValue::Float(x), JValue::Float(y)],
        ) {
            error!("webview: webViewTap failed: {:?}", e);
        }
    });
}

/// Scroll the page. `dx`/`dy` are deltas; `x`/`y` the normalized (0..1) focus point.
pub fn inject_scroll(app: &AndroidApp, dx: f32, dy: f32, x: f32, y: f32) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "webViewScroll", "(FFFF)V",
            &[JValue::Float(dx), JValue::Float(dy), JValue::Float(x), JValue::Float(y)],
        ) {
            error!("webview: webViewScroll failed: {:?}", e);
        }
    });
}

/// Type a string into the focused web input field.
pub fn type_text(app: &AndroidApp, text: &str) {
    with_activity(app, |env, activity| {
        let j = match env.new_string(text) { Ok(s) => s, Err(_) => return };
        if let Err(e) = env.call_method(
            activity, "webViewTypeText", "(Ljava/lang/String;)V",
            &[JValue::Object(&JObject::from(j))],
        ) {
            error!("webview: webViewTypeText failed: {:?}", e);
        }
    });
}

/// Instagram reel/post navigation: `next` = forward, else previous.
pub fn ig_navigate(app: &AndroidApp, next: bool) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "webViewIgNavigate", "(Z)V", &[JValue::Bool(next as u8)],
        ) {
            error!("webview: webViewIgNavigate failed: {:?}", e);
        }
    });
}

// ── Tabs ────────────────────────────────────────────────────────────────────────

/// Open a new tab (on the active engine).
pub fn new_tab(app: &AndroidApp) { call_void(app, "webViewNewTab"); }

/// Switch tab by delta (+1 next, -1 previous), wrapping.
pub fn switch_tab(app: &AndroidApp, delta: i32) {
    with_activity(app, |env, activity| {
        if let Err(e) = env.call_method(
            activity, "webViewSwitchTab", "(I)V", &[JValue::Int(delta)],
        ) {
            error!("webview: webViewSwitchTab failed: {:?}", e);
        }
    });
}

/// Close the active tab (keeps at least one).
pub fn close_tab(app: &AndroidApp) { call_void(app, "webViewCloseTab"); }

// ── Voice search (PS5 mic) ──────────────────────────────────────────────────────

/// Start PS5-mic voice search (Java SpeechRecognizer → `onVoiceResult`).
pub fn start_voice_search(app: &AndroidApp) { call_void(app, "startVoiceSearch"); }

// ── Consumers (called from the render loop) ─────────────────────────────────────

/// Take the latest captured browser frame, if any: (width, height, RGBA).
pub fn get_frame() -> Option<(u32, u32, Vec<u8>)> {
    WEB_FRAME.lock().ok().and_then(|mut f| f.take())
}

/// Take the latest voice-search transcript, if any.
pub fn take_voice_result() -> Option<String> {
    VOICE_RESULT.lock().ok().and_then(|mut v| v.take())
}

// ── Java → Rust JNI callbacks ───────────────────────────────────────────────────

/// Java pushes a captured RGBA page frame here.
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onWebFrame(
    mut env: jni::JNIEnv,
    _class: JObject,
    width: jni::sys::jint,
    height: jni::sys::jint,
    pixels: jni::objects::JByteArray,
) {
    let len = match env.get_array_length(&pixels) {
        Ok(l) if l > 0 => l as usize,
        _ => return,
    };

    let mut buf = take_buf(len);
    // Read the Java byte[] directly into our (recycled) buffer. get_byte_array_region
    // wants &mut [i8]; reinterpret the u8 slice to avoid a second allocation + copy.
    let dst: &mut [i8] = std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut i8, len);
    if env.get_byte_array_region(&pixels, 0, dst).is_err() {
        recycle(buf);
        return;
    }

    if let Ok(mut frame) = WEB_FRAME.lock() {
        // Recycle any frame the render loop never consumed.
        if let Some((_, _, old)) = frame.take() {
            recycle(old);
        }
        *frame = Some((width as u32, height as u32, buf));
    } else {
        recycle(buf);
    }
}

/// Java delivers a voice-search transcript.
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onVoiceResult(
    mut env: jni::JNIEnv,
    _class: JObject,
    text: jni::objects::JString,
) {
    if let Ok(s) = env.get_string(&text) {
        let s: String = s.into();
        info!("webview: voice result = {}", s);
        if let Ok(mut v) = VOICE_RESULT.lock() {
            *v = Some(s);
        }
    }
}

/// Java reports a voice-search error/cancel.
#[no_mangle]
pub unsafe extern "C" fn Java_com_vrapp_core_MainActivity_onVoiceError(
    _env: jni::JNIEnv,
    _class: JObject,
    code: jni::sys::jint,
) {
    error!("webview: voice search error code = {}", code);
}
