//! NDK Video Decoder Module
//!
//! Pure NDK video decoding using AMediaCodec and AMediaExtractor.
//! No Java, no JNI - just Rust + NDK.

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::ffi::CString;
use std::ptr;
use log::{info, warn, error};

/// Shared frame buffer for passing decoded frames to renderer
pub struct FrameBuffer {
    pub y_data: Vec<u8>,
    pub uv_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp_us: i64,
    pub has_new_frame: bool,
}

/// Playback state shared between decoder thread and main thread
pub struct PlaybackState {
    pub is_playing: bool,
    pub position_us: i64,
    pub duration_us: i64,
    pub seek_request: Option<i64>,
    pub pause_time_ms: u128,   // When pause started (elapsed ms)
    pub pause_offset_ms: u128, // Cumulative paused time
}

/// NDK-based video decoder using AMediaCodec
pub struct NdkVideoDecoder {
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    playback_state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
    decoder_thread: Option<JoinHandle<()>>,
}

impl NdkVideoDecoder {
    pub fn new() -> Self {
        Self {
            frame_buffer: Arc::new(Mutex::new(FrameBuffer {
                y_data: Vec::new(),
                uv_data: Vec::new(),
                width: 0,
                height: 0,
                timestamp_us: 0,
                has_new_frame: false,
            })),
            playback_state: Arc::new(Mutex::new(PlaybackState {
                is_playing: false,
                position_us: 0,
                duration_us: 0,
                seek_request: None,
                pause_time_ms: 0,
                pause_offset_ms: 0,
            })),
            running: Arc::new(AtomicBool::new(false)),
            decoder_thread: None,
        }
    }

    pub fn start(&mut self, file_path: &str) -> Result<(), String> {
        if self.running.load(Ordering::SeqCst) {
            self.stop();
        }

        info!("NdkVideoDecoder: Starting decode for {}", file_path);

        let frame_buffer = Arc::clone(&self.frame_buffer);
        let playback_state = Arc::clone(&self.playback_state);
        let running = Arc::clone(&self.running);
        let path = file_path.to_string();

        running.store(true, Ordering::SeqCst);

        if let Ok(mut state) = playback_state.lock() {
            state.is_playing = true;
        }

        self.decoder_thread = Some(thread::spawn(move || {
            if path.starts_with("test://") {
                run_test_pattern(frame_buffer, playback_state, running);
            } else {
                if let Err(e) = run_mediacodec_decode(&path, frame_buffer.clone(), playback_state.clone(), running.clone()) {
                    error!("MediaCodec decode error: {}", e);
                    // Fall back to test pattern
                    run_test_pattern(frame_buffer, playback_state, running);
                }
            }
        }));

        Ok(())
    }

    /// Start decoding from a file descriptor (for content:// URIs)
    pub fn start_from_fd(&mut self, fd: i32) -> Result<(), String> {
        if self.running.load(Ordering::SeqCst) {
            self.stop();
        }

        info!("NdkVideoDecoder: Starting decode from fd {}", fd);

        let frame_buffer = Arc::clone(&self.frame_buffer);
        let playback_state = Arc::clone(&self.playback_state);
        let running = Arc::clone(&self.running);

        running.store(true, Ordering::SeqCst);

        if let Ok(mut state) = playback_state.lock() {
            state.is_playing = true;
        }

        self.decoder_thread = Some(thread::spawn(move || {
            if let Err(e) = run_mediacodec_decode_fd(fd, frame_buffer.clone(), playback_state.clone(), running.clone()) {
                error!("MediaCodec decode fd error: {}", e);
                // Fall back to test pattern
                run_test_pattern(frame_buffer, playback_state, running);
            }
        }));

        Ok(())
    }

    pub fn get_frame(&self) -> Option<(Vec<u8>, Vec<u8>, u32, u32)> {
        if let Ok(mut buffer) = self.frame_buffer.lock() {
            if buffer.has_new_frame && !buffer.y_data.is_empty() {
                buffer.has_new_frame = false;
                return Some((buffer.y_data.clone(), buffer.uv_data.clone(), buffer.width, buffer.height));
            }
        }
        None
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn pause(&self) {
        if let Ok(mut state) = self.playback_state.lock() {
            state.is_playing = false;
        }
    }

    pub fn resume(&self) {
        if let Ok(mut state) = self.playback_state.lock() {
            state.is_playing = true;
        }
    }

    pub fn seek(&self, position_us: i64) {
        if let Ok(mut state) = self.playback_state.lock() {
            state.seek_request = Some(position_us);
            // Update position immediately so slider reflects seek even when paused
            state.position_us = position_us;
        }
    }

    pub fn get_position(&self) -> i64 {
        self.playback_state.lock().map(|s| s.position_us).unwrap_or(0)
    }

    pub fn get_duration(&self) -> i64 {
        self.playback_state.lock().map(|s| s.duration_us).unwrap_or(0)
    }

    pub fn is_paused(&self) -> bool {
        !self.playback_state.lock().map(|s| s.is_playing).unwrap_or(true)
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.decoder_thread.take() {
            let _ = handle.join();
        }
        if let Ok(mut buffer) = self.frame_buffer.lock() {
            buffer.y_data.clear();
            buffer.uv_data.clear();
            buffer.has_new_frame = false;
        }
    }
}

impl Drop for NdkVideoDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Test pattern generator (fallback)
fn run_test_pattern(
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    playback_state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
) {
    let width = 1280u32;
    let height = 720u32;
    let frame_size = (width * height * 4) as usize;

    if let Ok(mut state) = playback_state.lock() {
        state.duration_us = 60_000_000;
    }

    let start_time = std::time::Instant::now();
    let mut frame_count: u64 = 0;

    while running.load(Ordering::SeqCst) {
        let is_playing = playback_state.lock().map(|s| s.is_playing).unwrap_or(false);
        if !is_playing {
            thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let y_size = (width * height) as usize;
        let uv_size = y_size / 2;
        let mut y_data = vec![0u8; y_size];
        let mut uv_data = vec![128u8; uv_size]; // Grayscale

        let time_offset = ((frame_count * 4) % 256) as u8;
        
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width) + x) as usize;
                y_data[idx] = (x as u8).wrapping_add(time_offset).wrapping_add(y as u8);
            }
        }

        let elapsed_us = start_time.elapsed().as_micros() as i64;
        if let Ok(mut state) = playback_state.lock() {
            state.position_us = elapsed_us % state.duration_us;
        }

        if let Ok(mut buffer) = frame_buffer.lock() {
            buffer.y_data = y_data;
            buffer.uv_data = uv_data;
            buffer.width = width;
            buffer.height = height;
            buffer.timestamp_us = elapsed_us;
            buffer.has_new_frame = true;
        }
        
        frame_count += 1;
        thread::sleep(std::time::Duration::from_millis(16)); // ~60 FPS
    }
}

/// Real MediaCodec decoding via NDK
fn run_mediacodec_decode(
    file_path: &str,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    playback_state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
) -> Result<(), String> {
    use ndk_sys::*;
    
    info!("MediaCodec: Opening {}", file_path);

    // Open file
    let file = File::open(file_path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    let fd = file.as_raw_fd();
    let file_len = file.metadata().map(|m| m.len() as i64).unwrap_or(i64::MAX);

    unsafe {
        // Create extractor
        let extractor = AMediaExtractor_new();
        if extractor.is_null() {
            return Err("Failed to create AMediaExtractor".into());
        }

        // Set data source from file descriptor
        let status = AMediaExtractor_setDataSourceFd(extractor, fd, 0, file_len);
        if status.0 != 0 {
            AMediaExtractor_delete(extractor);
            return Err(format!("Failed to set data source: {:?}", status.0));
        }

        // Find video track
        let track_count = AMediaExtractor_getTrackCount(extractor);
        info!("MediaCodec: Found {} tracks", track_count);

        let mut video_track: Option<usize> = None;
        let mut video_format: *mut AMediaFormat = ptr::null_mut();
        let mut mime_type = String::new();

        for i in 0..track_count as usize {
            let format = AMediaExtractor_getTrackFormat(extractor, i);
            if format.is_null() { continue; }

            let mut mime_ptr: *const std::os::raw::c_char = ptr::null();
            let key = CString::new("mime").unwrap();
            if AMediaFormat_getString(format, key.as_ptr(), &mut mime_ptr) {
                if !mime_ptr.is_null() {
                    let mime = std::ffi::CStr::from_ptr(mime_ptr).to_string_lossy();
                    info!("Track {}: {}", i, mime);
                    if mime.starts_with("video/") {
                        video_track = Some(i);
                        video_format = format;
                        mime_type = mime.to_string();
                        break;
                    }
                }
            }
            AMediaFormat_delete(format);
        }

        let track_idx = video_track.ok_or("No video track found")?;
        if video_format.is_null() {
            AMediaExtractor_delete(extractor);
            return Err("No video format".into());
        }

        // Get dimensions
        let mut width: i32 = 1280;
        let mut height: i32 = 720;
        
        let key_width = CString::new("width").unwrap();
        let key_height = CString::new("height").unwrap();
        
        AMediaFormat_getInt32(video_format, key_width.as_ptr(), &mut width);
        AMediaFormat_getInt32(video_format, key_height.as_ptr(), &mut height);
        
        // Measure-and-Lock Variables
        let mut frames_for_estimation = 0;
        let mut previous_pts: i64 = -1;
        let mut accumulated_delta: i64 = 0;
        let mut samples_count: i64 = 0;
        let mut target_interval_ms: u64 = 33; // Start with 30fps assumption
        let mut next_frame_target = std::time::Instant::now();
        
        // Select track
        let status = AMediaExtractor_selectTrack(extractor, track_idx);
        if status.0 != 0 {
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            // Close the fd since we own it
            libc::close(fd);
            return Err(format!("Failed to select track: {:?}", status.0));
        }

        // Create decoder
        let mime_cstr = CString::new(mime_type.clone()).unwrap();
        let codec = AMediaCodec_createDecoderByType(mime_cstr.as_ptr());
        if codec.is_null() {
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            return Err("Failed to create decoder".into());
        }

        // Configure decoder (no surface - raw output)
        let status = AMediaCodec_configure(codec, video_format, ptr::null_mut(), ptr::null_mut(), 0);
        if status.0 != 0 {
            AMediaCodec_delete(codec);
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            return Err(format!("Failed to configure decoder: {:?}", status.0));
        }

        // Start decoder
        let status = AMediaCodec_start(codec);
        if status.0 != 0 {
            AMediaCodec_delete(codec);
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            return Err(format!("Failed to start decoder: {:?}", status.0));
        }

        info!("MediaCodec: Decoder started successfully");

        // Decode loop
        let start_time = std::time::Instant::now();
        let mut eos_input = false;
        let mut frame_count: u64 = 0;

        while running.load(Ordering::SeqCst) {
            // Check pause
            let is_playing = playback_state.lock().map(|s| s.is_playing).unwrap_or(false);
            if !is_playing {
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }

            // Handle seek
            if let Ok(mut state) = playback_state.lock() {
                if let Some(seek_pos) = state.seek_request.take() {
                    AMediaExtractor_seekTo(extractor, seek_pos, SeekMode::AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC);
                    AMediaCodec_flush(codec);
                    eos_input = false;
                }
            }

            // Feed input
            if !eos_input {
                let input_idx = AMediaCodec_dequeueInputBuffer(codec, 5000);
                if input_idx >= 0 {
                    let mut buf_size: usize = 0;
                    let input_buf = AMediaCodec_getInputBuffer(codec, input_idx as usize, &mut buf_size);
                    
                    if !input_buf.is_null() && buf_size > 0 {
                        let sample_size = AMediaExtractor_readSampleData(
                            extractor, 
                            input_buf, 
                            buf_size
                        );
                        
                        if sample_size >= 0 {
                            let pts = AMediaExtractor_getSampleTime(extractor);
                            let flags = AMediaExtractor_getSampleFlags(extractor);
                            
                            AMediaCodec_queueInputBuffer(
                                codec, 
                                input_idx as usize, 
                                0, 
                                sample_size as usize, 
                                pts as u64, 
                                flags as u32
                            );
                            AMediaExtractor_advance(extractor);
                        } else {
                            // EOS - loop video
                            AMediaExtractor_seekTo(extractor, 0, SeekMode::AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC);
                        }
                    }
                }
            }

            // Get output
            let mut buffer_info = AMediaCodecBufferInfo {
                offset: 0,
                size: 0,
                presentationTimeUs: 0,
                flags: 0,
            };
            
            let output_idx = AMediaCodec_dequeueOutputBuffer(codec, &mut buffer_info, 5000);
            
            if output_idx >= 0 {
                let pts = buffer_info.presentationTimeUs;
                
                if let Ok(mut state) = playback_state.lock() {
                    state.position_us = pts;
                }

                // Get output buffer
                let mut out_size: usize = 0;
                let out_buf = AMediaCodec_getOutputBuffer(codec, output_idx as usize, &mut out_size);
                
                if !out_buf.is_null() && out_size > 0 {
                    let yuv_data = std::slice::from_raw_parts(out_buf, out_size);
                    let rgba = convert_yuv_to_rgba(yuv_data, width as u32, height as u32);
                    
                    if let Ok(mut buffer) = frame_buffer.lock() {
                        // buffer.data = rgba;
                        // Legacy path disabled - just satisfy type checker
                        buffer.y_data.resize((width as u32 * height as u32) as usize, 0); 
                        buffer.uv_data.resize((width as u32 * height as u32 / 2) as usize, 128);
                        buffer.width = width as u32;
                        buffer.height = height as u32;
                        buffer.timestamp_us = pts;
                        buffer.has_new_frame = true;
                    }
                }

                AMediaCodec_releaseOutputBuffer(codec, output_idx as usize, false);
                
                // Simple frame pacing: ~16ms for 60fps, ~33ms for 30fps
                // This avoids wall-clock drift issues with pause/resume
                thread::sleep(std::time::Duration::from_millis(33));

                frame_count += 1;
                if frame_count % 100 == 0 {
                    info!("MediaCodec: Decoded {} frames", frame_count);
                }
            }
        }

        // Cleanup
        AMediaCodec_stop(codec);
        AMediaCodec_delete(codec);
        AMediaFormat_delete(video_format);
        AMediaExtractor_delete(extractor);

        info!("MediaCodec: Stopped after {} frames", frame_count);
    }

    Ok(())
}

/// Real MediaCodec decoding via NDK from file descriptor
fn run_mediacodec_decode_fd(
    fd: i32,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    playback_state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
) -> Result<(), String> {
    use ndk_sys::*;
    
    info!("MediaCodec: Opening from fd {}", fd);

    // We pass i64::MAX for file length since we don't know the size from fd alone
    // AMediaExtractor will figure it out
    let file_len = i64::MAX;

    unsafe {
        let extractor = AMediaExtractor_new();
        if extractor.is_null() {
            return Err("Failed to create AMediaExtractor".into());
        }

        let status = AMediaExtractor_setDataSourceFd(extractor, fd, 0, file_len);
        if status.0 != 0 {
            AMediaExtractor_delete(extractor);
            // Close the fd since we own it
            libc::close(fd);
            return Err(format!("Failed to set data source fd: {:?}", status.0));
        }

        let track_count = AMediaExtractor_getTrackCount(extractor);
        info!("MediaCodec: Found {} tracks from fd", track_count);

        let mut video_track: Option<usize> = None;
        let mut video_format: *mut AMediaFormat = ptr::null_mut();
        let mut mime_type = String::new();

        for i in 0..track_count as usize {
            let format = AMediaExtractor_getTrackFormat(extractor, i);
            if format.is_null() { continue; }

            let mut mime_ptr: *const std::os::raw::c_char = ptr::null();
            let key = CString::new("mime").unwrap();
            if AMediaFormat_getString(format, key.as_ptr(), &mut mime_ptr) {
                if !mime_ptr.is_null() {
                    let mime = std::ffi::CStr::from_ptr(mime_ptr).to_string_lossy();
                    info!("Track {}: {}", i, mime);
                    if mime.starts_with("video/") {
                        video_track = Some(i);
                        video_format = format;
                        mime_type = mime.to_string();
                        break;
                    }
                }
            }
            AMediaFormat_delete(format);
        }

        let track_idx = video_track.ok_or("No video track found")?;
        if video_format.is_null() {
            AMediaExtractor_delete(extractor);
            libc::close(fd);
            return Err("No video format".into());
        }

        let mut width: i32 = 1280;
        let mut height: i32 = 720;
        let mut duration: i64 = 0;
        
        let key_width = CString::new("width").unwrap();
        let key_height = CString::new("height").unwrap();
        let key_duration = CString::new("durationUs").unwrap();
        
        AMediaFormat_getInt32(video_format, key_width.as_ptr(), &mut width);
        AMediaFormat_getInt32(video_format, key_height.as_ptr(), &mut height);
        AMediaFormat_getInt64(video_format, key_duration.as_ptr(), &mut duration);

        info!("MediaCodec: Video {}x{}, duration {}us, mime {}", width, height, duration, mime_type);

        if let Ok(mut state) = playback_state.lock() {
            state.duration_us = duration;
        }

        let status = AMediaExtractor_selectTrack(extractor, track_idx);
        if status.0 != 0 {
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            libc::close(fd);
            return Err(format!("Failed to select track: {:?}", status.0));
        }

        let mime_cstr = CString::new(mime_type.clone()).unwrap();
        let codec = AMediaCodec_createDecoderByType(mime_cstr.as_ptr());
        if codec.is_null() {
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            libc::close(fd);
            return Err("Failed to create MediaCodec".into());
        }

        let status = AMediaCodec_configure(codec, video_format, ptr::null_mut(), ptr::null_mut(), 0);
        if status.0 != 0 {
            AMediaCodec_delete(codec);
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            libc::close(fd);
            return Err(format!("Failed to configure codec: {:?}", status.0));
        }

        let status = AMediaCodec_start(codec);
        if status.0 != 0 {
            AMediaCodec_delete(codec);
            AMediaFormat_delete(video_format);
            AMediaExtractor_delete(extractor);
            libc::close(fd);
            return Err(format!("Failed to start codec: {:?}", status.0));
        }

        info!("MediaCodec: Decoder started successfully from fd");

        let mut start_time = std::time::Instant::now();
        let mut total_paused_duration = std::time::Duration::from_millis(0);
        let mut last_pause_check = std::time::Instant::now();
        let mut frame_count: u64 = 0;
        let mut first_frame = true;

        // Measure-and-Lock Variables
        let mut frames_for_estimation = 0;
        let mut previous_pts: i64 = -1;
        let mut accumulated_delta: i64 = 0;
        let mut samples_count: i64 = 0;
        let mut target_interval_ms: u64 = 33; // Start with 30fps assumption
        let mut next_frame_target = std::time::Instant::now();

        while running.load(Ordering::SeqCst) {
            let is_playing = playback_state.lock().map(|s| s.is_playing).unwrap_or(false);
            
            if !is_playing {
                thread::sleep(std::time::Duration::from_millis(10));
                // Accumulate paused duration
                total_paused_duration += last_pause_check.elapsed();
                last_pause_check = std::time::Instant::now();
                continue;
            }
            last_pause_check = std::time::Instant::now();

            if let Ok(mut state) = playback_state.lock() {
                if let Some(seek_pos) = state.seek_request.take() {
                    AMediaExtractor_seekTo(extractor, seek_pos, SeekMode::AMEDIAEXTRACTOR_SEEK_CLOSEST_SYNC);
                    AMediaCodec_flush(codec);
                    
                    // Reset timing after seek
                    start_time = std::time::Instant::now();
                    total_paused_duration = std::time::Duration::from_millis(0);
                    
                    // Adjust start_time so that (now - start) approx matches seek_pos
                    if let Some(adjusted_start) = start_time.checked_sub(std::time::Duration::from_micros(seek_pos as u64)) {
                        start_time = adjusted_start;
                    }
                }
            }

            let input_idx = AMediaCodec_dequeueInputBuffer(codec, 5000);
            if input_idx >= 0 {
                let mut buf_size: usize = 0;
                let input_buf = AMediaCodec_getInputBuffer(codec, input_idx as usize, &mut buf_size);
                
                if !input_buf.is_null() && buf_size > 0 {
                    let sample_size = AMediaExtractor_readSampleData(extractor, input_buf, buf_size);
                    
                    if sample_size >= 0 {
                        let pts = AMediaExtractor_getSampleTime(extractor);
                        let flags = AMediaExtractor_getSampleFlags(extractor);
                        
                        AMediaCodec_queueInputBuffer(
                            codec, input_idx as usize, 0, 
                            sample_size as usize, pts as u64, flags as u32
                        );
                        AMediaExtractor_advance(extractor);
                    } else {
                        AMediaExtractor_seekTo(extractor, 0, SeekMode::AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC);
                    }
                }
            }

            let mut buffer_info = AMediaCodecBufferInfo {
                offset: 0, size: 0, presentationTimeUs: 0, flags: 0,
            };
            
            let output_idx = AMediaCodec_dequeueOutputBuffer(codec, &mut buffer_info, 5000);
            
            if output_idx >= 0 {
                let pts = buffer_info.presentationTimeUs;
                
                if let Ok(mut state) = playback_state.lock() {
                    state.position_us = pts;
                }

                let mut out_size: usize = 0;
                let out_buf = AMediaCodec_getOutputBuffer(codec, output_idx as usize, &mut out_size);
                
                if !out_buf.is_null() && out_size > 0 {
                    let src_slice = std::slice::from_raw_parts(out_buf, out_size);
                    let y_size = (width * height) as usize;
                    let uv_size = y_size / 2;
                    
                    if let Ok(mut buffer) = frame_buffer.lock() {
                        if buffer.y_data.len() != y_size { buffer.y_data.resize(y_size, 0); }
                        if buffer.uv_data.len() != uv_size { buffer.uv_data.resize(uv_size, 0); }
                        
                        // Safety check for buffer size
                        if src_slice.len() >= y_size + uv_size {
                            buffer.y_data.copy_from_slice(&src_slice[0..y_size]);
                            buffer.uv_data.copy_from_slice(&src_slice[y_size..y_size+uv_size]);
                            buffer.width = width as u32;
                            buffer.height = height as u32;
                            buffer.timestamp_us = pts;
                            buffer.has_new_frame = true;
                        }
                    }
                }
                
                AMediaCodec_releaseOutputBuffer(codec, output_idx as usize, false);
                
                // Measure-and-Lock Pacing Strategy
                // 1. Measure actual frame rate from first 15 frames
                if frames_for_estimation < 15 {
                    if previous_pts >= 0 {
                        let delta = (pts - previous_pts) / 1000;
                        if delta > 0 {
                           accumulated_delta += delta;
                           samples_count += 1;
                        }
                    }
                    previous_pts = pts;
                    frames_for_estimation += 1;
                    
                    // Default to 30fps (33ms) during estimation to avoid super fast playback
                    thread::sleep(std::time::Duration::from_millis(33));
                    
                    if frames_for_estimation == 15 && samples_count > 0 {
                        let avg_delta = accumulated_delta as f64 / samples_count as f64;
                        target_interval_ms = avg_delta.round() as u64;
                        info!("MediaCodec: Detected Fixed Frame Rate. Avg Delta: {:.2}ms. Locking to {}ms", avg_delta, target_interval_ms);
                        next_frame_target = std::time::Instant::now();
                    }
                } else {
                    // 2. Locked Constant Timing Loop
                    // Advance target time by fixed interval
                    next_frame_target += std::time::Duration::from_millis(target_interval_ms);
                    
                    let now = std::time::Instant::now();
                    if next_frame_target > now {
                        thread::sleep(next_frame_target - now);
                    } else {
                        // We are behind. If we are WAY behind (>100ms), reset the clock to avoid seeking frenzy
                        if now.duration_since(next_frame_target).as_millis() > 100 {
                             next_frame_target = now;
                        }
                    }
                }

                frame_count += 1;
                if frame_count % 60 == 0 {
                    // info!("MediaCodec: Decoded {} frames (Locked: {}ms)", frame_count, target_interval_ms);
                }
            }
        }

        AMediaCodec_stop(codec);
        AMediaCodec_delete(codec);
        AMediaFormat_delete(video_format);
        AMediaExtractor_delete(extractor);
        libc::close(fd);

        info!("MediaCodec fd: Stopped after {} frames", frame_count);
    }

    Ok(())
}

/// Convert YUV420 (NV12/NV21) to RGBA
fn convert_yuv_to_rgba(yuv: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let frame_size = w * h;
    
    if yuv.len() < frame_size + frame_size / 2 {
        return vec![128u8; w * h * 4];
    }

    let mut rgba = vec![0u8; w * h * 4];
    
    for y in 0..h {
        for x in 0..w {
            let y_idx = y * w + x;
            let uv_idx = frame_size + (y / 2) * w + (x / 2) * 2;
            
            let y_val = yuv[y_idx] as i32;
            let u_val = yuv.get(uv_idx).copied().unwrap_or(128) as i32;
            let v_val = yuv.get(uv_idx + 1).copied().unwrap_or(128) as i32;

            let r = (y_val + ((351 * (v_val - 128)) >> 8)).clamp(0, 255) as u8;
            let g = (y_val - ((86 * (u_val - 128) + 179 * (v_val - 128)) >> 8)).clamp(0, 255) as u8;
            let b = (y_val + ((443 * (u_val - 128)) >> 8)).clamp(0, 255) as u8;

            let idx = (y * w + x) * 4;
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }

    rgba
}

/// List video files in a directory (pure Rust, no Java)
pub fn list_video_files(directory: &str) -> Vec<String> {
    let mut videos = Vec::new();
    
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                let lower = name.to_lowercase();
                if lower.ends_with(".mp4") || lower.ends_with(".mkv") || 
                   lower.ends_with(".webm") || lower.ends_with(".avi") {
                    if let Some(path) = entry.path().to_str() {
                        videos.push(path.to_string());
                    }
                }
            }
        }
    }
    
    videos
}
