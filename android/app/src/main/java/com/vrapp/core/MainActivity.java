package com.vrapp.core;

import android.app.NativeActivity;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.os.ParcelFileDescriptor;
import android.util.Log;
import android.graphics.Bitmap;
import android.media.MediaMetadataRetriever;
import android.media.MediaPlayer;
import android.media.AudioManager;
import android.view.MotionEvent;
import android.view.InputDevice;

import java.io.FileNotFoundException;

public class MainActivity extends NativeActivity {
    private static final String TAG = "VRAppJava";
    private static final int PICK_VIDEO_REQUEST = 1001;
    private static final int MAX_WIDTH = 854;

    // Keep screen on
    private PowerManager.WakeLock wakeLock;

    // Audio Player
    private MediaPlayer mediaPlayer;

    // Video Frames - reuse buffers
    private MediaMetadataRetriever retriever;
    private byte[] frameBuffer;
    private int[] pixelBuffer;
    private volatile int frameWidth = 640;
    private volatile int frameHeight = 360;
    private volatile boolean hasVideo = false;
    private Thread frameThread;
    private volatile boolean isRunning = false;
    private final Object lock = new Object();

    // For NDK decoder
    private Uri currentVideoUri = null;
    private ParcelFileDescriptor currentPfd = null;

    static {
        System.loadLibrary("vr_core");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Acquire wake lock
        PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
        wakeLock = pm.newWakeLock(PowerManager.SCREEN_BRIGHT_WAKE_LOCK | PowerManager.ACQUIRE_CAUSES_WAKEUP,
                "vrapp:wakelock");
        wakeLock.acquire();

        Log.i(TAG, "MainActivity created - wake lock acquired");
    }

    // D-pad state tracking for volume control
    private float lastHatX = 0f;
    private float lastHatY = 0f;
    private long lastVolumeChangeTime = 0;

    // Called from Rust to check D-pad and adjust volume
    // Since NativeActivity consumes gamepad events, we poll via physical button
    // simulation
    public void checkVolumeButtons(boolean left, boolean right) {
        long now = System.currentTimeMillis();
        // Debounce: only change volume every 200ms
        if (now - lastVolumeChangeTime < 200)
            return;

        if (left) {
            volumeDown();
            lastVolumeChangeTime = now;
            Log.i(TAG, "Volume DOWN via native D-pad");
        }
        if (right) {
            volumeUp();
            lastVolumeChangeTime = now;
            Log.i(TAG, "Volume UP via native D-pad");
        }
    }

    // Poll D-pad HAT axis state and adjust volume (called from Rust render loop)
    // This bypasses NativeActivity's input consumption
    private int cachedGamepadDeviceId = -1;

    public void pollDpadForVolume() {
        // Find gamepad if not cached
        if (cachedGamepadDeviceId == -1) {
            int[] deviceIds = android.view.InputDevice.getDeviceIds();
            for (int id : deviceIds) {
                android.view.InputDevice device = android.view.InputDevice.getDevice(id);
                if (device != null &&
                        (device.getSources()
                                & android.view.InputDevice.SOURCE_GAMEPAD) == android.view.InputDevice.SOURCE_GAMEPAD) {
                    cachedGamepadDeviceId = id;
                    Log.i(TAG, "Found gamepad: " + device.getName() + " id=" + id);
                    break;
                }
            }
        }

        if (cachedGamepadDeviceId == -1)
            return;

        // Unfortunately InputDevice API doesn't give us current axis values
        // We can only get motion ranges, not live values
        // Need to use View.onGenericMotionEvent or InputEventReceiver instead
        // For now, this won't work - keeping for reference
    }

    // Alternative: Use InputEventReceiver in a Looper thread
    // This is complex - simpler to fix native input handling

    // Volume Control Methods
    public void volumeUp() {
        AudioManager audioManager = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        int current = audioManager.getStreamVolume(AudioManager.STREAM_MUSIC);
        int max = audioManager.getStreamMaxVolume(AudioManager.STREAM_MUSIC);
        if (current < max) {
            audioManager.setStreamVolume(AudioManager.STREAM_MUSIC, current + 1, AudioManager.FLAG_SHOW_UI);
        }
        Log.i(TAG, "Volume Up: " + (current + 1) + "/" + max);
    }

    public void volumeDown() {
        AudioManager audioManager = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        int current = audioManager.getStreamVolume(AudioManager.STREAM_MUSIC);
        int max = audioManager.getStreamMaxVolume(AudioManager.STREAM_MUSIC);
        if (current > 0) {
            audioManager.setStreamVolume(AudioManager.STREAM_MUSIC, current - 1, AudioManager.FLAG_SHOW_UI);
        }
        Log.i(TAG, "Volume Down: " + (current - 1) + "/" + max);
    }

    public int getVolume() {
        AudioManager audioManager = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        return audioManager.getStreamVolume(AudioManager.STREAM_MUSIC);
    }

    public int getMaxVolume() {
        AudioManager audioManager = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        return audioManager.getStreamMaxVolume(AudioManager.STREAM_MUSIC);
    }

    public void launchVideoPicker() {
        Log.i(TAG, "Launching Video Picker (Google Photos)...");
        // ACTION_GET_CONTENT shows all content providers including Google Photos
        Intent intent = new Intent(Intent.ACTION_GET_CONTENT);
        intent.setType("video/*");
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        // Allow picking from external apps like Google Photos
        intent.putExtra(Intent.EXTRA_LOCAL_ONLY, false);
        startActivityForResult(Intent.createChooser(intent, "Select Video"), PICK_VIDEO_REQUEST);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        Log.i(TAG, "onActivityResult: req=" + requestCode + " res=" + resultCode);

        if (requestCode == PICK_VIDEO_REQUEST && resultCode == RESULT_OK) {
            if (data != null && data.getData() != null) {
                Uri uri = data.getData();
                try {
                    getContentResolver().takePersistableUriPermission(uri,
                            Intent.FLAG_GRANT_READ_URI_PERMISSION);
                } catch (SecurityException e) {
                    Log.w(TAG, "Failed to persist permission: " + e);
                }

                Log.i(TAG, "Selected Video URI: " + uri);
                currentVideoUri = uri;

                // Get file descriptor for NDK decoder
                int fd = getVideoFd();
                if (fd >= 0) {
                    Log.i(TAG, "Got file descriptor: " + fd);
                    onVideoFdReady(fd);
                }

                startVideo(uri);
                onVideoPicked(uri.toString());
            }
        }
    }

    // Get file descriptor from current content:// URI
    public int getVideoFd() {
        if (currentVideoUri == null) {
            return -1;
        }
        try {
            // Close previous fd if any
            if (currentPfd != null) {
                try {
                    currentPfd.close();
                } catch (Exception e) {
                }
            }
            currentPfd = getContentResolver().openFileDescriptor(currentVideoUri, "r");
            if (currentPfd != null) {
                return currentPfd.detachFd(); // Transfer ownership to native
            }
        } catch (FileNotFoundException e) {
            Log.e(TAG, "Failed to open file: " + e);
        }
        return -1;
    }

    private void startVideo(Uri uri) {
        stopVideo();

        // Start Audio
        try {
            mediaPlayer = new MediaPlayer();
            mediaPlayer.setDataSource(this, uri);
            mediaPlayer.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
                @Override
                public void onPrepared(MediaPlayer mp) {
                    Log.i(TAG, "Audio ready");
                    mp.start();
                    mp.setLooping(true);
                }
            });
            mediaPlayer.prepareAsync();
        } catch (Exception e) {
            Log.e(TAG, "Audio failed: " + e);
        }

        // Setup frame retriever
        try {
            retriever = new MediaMetadataRetriever();
            retriever.setDataSource(this, uri);

            String widthStr = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH);
            String heightStr = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT);

            int w = 640, h = 360;
            if (widthStr != null)
                w = Integer.parseInt(widthStr);
            if (heightStr != null)
                h = Integer.parseInt(heightStr);

            // Scale down for memory
            if (w > MAX_WIDTH) {
                float scale = (float) MAX_WIDTH / w;
                w = MAX_WIDTH;
                h = (int) (h * scale);
            }

            frameWidth = w;
            frameHeight = h;

            // Pre-allocate buffers
            int pixelCount = frameWidth * frameHeight;
            pixelBuffer = new int[pixelCount];
            frameBuffer = new byte[pixelCount * 4];

            Log.i(TAG, "Video: " + frameWidth + "x" + frameHeight);
            hasVideo = true;

            // Start frame extraction thread
            isRunning = true;
            frameThread = new Thread(new Runnable() {
                @Override
                public void run() {
                    extractFrames();
                }
            }, "FrameExtractor");
            frameThread.start();

        } catch (Exception e) {
            Log.e(TAG, "Retriever failed: " + e);
            hasVideo = false;
        }
    }

    private void extractFrames() {
        Bitmap scaledBitmap = null;

        while (isRunning && mediaPlayer != null && retriever != null) {
            try {
                if (!mediaPlayer.isPlaying()) {
                    Thread.sleep(50);
                    continue;
                }

                int posMs = mediaPlayer.getCurrentPosition();
                long posUs = posMs * 1000L;

                Bitmap frame = retriever.getFrameAtTime(posUs, MediaMetadataRetriever.OPTION_CLOSEST);
                if (frame != null) {
                    // Scale if needed
                    if (frame.getWidth() != frameWidth || frame.getHeight() != frameHeight) {
                        if (scaledBitmap == null || scaledBitmap.getWidth() != frameWidth) {
                            if (scaledBitmap != null)
                                scaledBitmap.recycle();
                            scaledBitmap = Bitmap.createBitmap(frameWidth, frameHeight, Bitmap.Config.ARGB_8888);
                        }
                        android.graphics.Canvas canvas = new android.graphics.Canvas(scaledBitmap);
                        android.graphics.Rect src = new android.graphics.Rect(0, 0, frame.getWidth(),
                                frame.getHeight());
                        android.graphics.Rect dst = new android.graphics.Rect(0, 0, frameWidth, frameHeight);
                        canvas.drawBitmap(frame, src, dst, null);
                        frame.recycle();
                        frame = scaledBitmap;
                    }

                    // Get pixels into reusable buffer
                    synchronized (lock) {
                        frame.getPixels(pixelBuffer, 0, frameWidth, 0, 0, frameWidth, frameHeight);

                        // Convert ARGB to RGBA in-place
                        for (int i = 0; i < pixelBuffer.length; i++) {
                            int p = pixelBuffer[i];
                            int idx = i * 4;
                            frameBuffer[idx] = (byte) ((p >> 16) & 0xFF); // R
                            frameBuffer[idx + 1] = (byte) ((p >> 8) & 0xFF); // G
                            frameBuffer[idx + 2] = (byte) (p & 0xFF); // B
                            frameBuffer[idx + 3] = (byte) 255; // A
                        }
                    }

                    if (frame != scaledBitmap) {
                        frame.recycle();
                    }
                }

                // ~15 FPS - stable
                Thread.sleep(66);

            } catch (Exception e) {
                Log.e(TAG, "Frame error: " + e.getMessage());
                try {
                    Thread.sleep(100);
                } catch (Exception ex) {
                }
            }
        }

        if (scaledBitmap != null) {
            scaledBitmap.recycle();
        }
    }

    private void stopVideo() {
        isRunning = false;
        hasVideo = false;

        if (frameThread != null) {
            try {
                frameThread.join(500);
            } catch (Exception e) {
            }
            frameThread = null;
        }

        if (mediaPlayer != null) {
            try {
                mediaPlayer.release();
            } catch (Exception e) {
            }
            mediaPlayer = null;
        }

        if (retriever != null) {
            try {
                retriever.release();
            } catch (Exception e) {
            }
            retriever = null;
        }
    }

    // Audio control methods for Rust JNI

    // Start audio from a file path (called by file browser)
    public void startAudioFromPath(String filePath) {
        Log.i(TAG, "startAudioFromPath: " + filePath);

        // Stop existing audio
        if (mediaPlayer != null) {
            try {
                mediaPlayer.release();
            } catch (Exception e) {
            }
            mediaPlayer = null;
        }

        try {
            mediaPlayer = new MediaPlayer();
            mediaPlayer.setDataSource(filePath);
            mediaPlayer.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
                @Override
                public void onPrepared(MediaPlayer mp) {
                    Log.i(TAG, "Audio ready from path");
                    mp.start();
                    mp.setLooping(true);
                }
            });
            mediaPlayer.prepareAsync();
        } catch (Exception e) {
            Log.e(TAG, "startAudioFromPath failed: " + e);
        }
    }

    public void pauseAudio() {
        if (mediaPlayer != null && mediaPlayer.isPlaying()) {
            try {
                mediaPlayer.pause();
                Log.i(TAG, "Audio paused");
            } catch (Exception e) {
                Log.e(TAG, "pauseAudio failed: " + e);
            }
        }
    }

    public void resumeAudio() {
        if (mediaPlayer != null && !mediaPlayer.isPlaying()) {
            try {
                mediaPlayer.start();
                Log.i(TAG, "Audio resumed");
            } catch (Exception e) {
                Log.e(TAG, "resumeAudio failed: " + e);
            }
        }
    }

    public void seekAudio(int positionMs) {
        if (mediaPlayer != null) {
            try {
                mediaPlayer.seekTo(positionMs);
                Log.i(TAG, "Audio seek to " + positionMs + "ms");
            } catch (Exception e) {
                Log.e(TAG, "seekAudio failed: " + e);
            }
        }
    }

    // JNI methods
    public byte[] getVideoFrame() {
        if (hasVideo && frameBuffer != null) {
            synchronized (lock) {
                return frameBuffer;
            }
        }
        return null;
    }

    public int getVideoWidth() {
        return frameWidth;
    }

    public int getVideoHeight() {
        return frameHeight;
    }

    @Override
    protected void onDestroy() {
        stopVideo();
        if (wakeLock != null && wakeLock.isHeld()) {
            wakeLock.release();
        }
        super.onDestroy();
    }

    public native void onVideoPicked(String uri);

    public native void onVideoFdReady(int fd);

    // Native methods for gamepad input
    public native void onGamepadButton(int buttonCode, boolean pressed);

    public native void onGamepadAxis(float leftX, float leftY, float rightX, float rightY, float l2, float r2);

    @Override
    public boolean dispatchKeyEvent(android.view.KeyEvent event) {
        // Check if this is a gamepad/joystick source
        int source = event.getSource();
        if ((source & android.view.InputDevice.SOURCE_GAMEPAD) == android.view.InputDevice.SOURCE_GAMEPAD ||
                (source & android.view.InputDevice.SOURCE_JOYSTICK) == android.view.InputDevice.SOURCE_JOYSTICK) {

            int keyCode = event.getKeyCode();
            boolean pressed = (event.getAction() == android.view.KeyEvent.ACTION_DOWN);

            Log.i(TAG, "GAMEPAD KEY: code=" + keyCode + " pressed=" + pressed);

            // D-pad handling for volume (KeyEvent codes 19-22)
            if (pressed) {
                switch (keyCode) {
                    case android.view.KeyEvent.KEYCODE_DPAD_LEFT:
                        volumeDown();
                        Log.i(TAG, "D-pad LEFT KeyEvent: Volume Down");
                        break;
                    case android.view.KeyEvent.KEYCODE_DPAD_RIGHT:
                        volumeUp();
                        Log.i(TAG, "D-pad RIGHT KeyEvent: Volume Up");
                        break;
                }
            }

            // Call Rust native method
            onGamepadButton(keyCode, pressed);

            // Don't consume - let system also handle
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(android.view.MotionEvent event) {
        Log.i(TAG, "dispatchGenericMotionEvent CALLED! source=" + event.getSource());

        // Check if this is a joystick/gamepad motion
        int source = event.getSource();
        if ((source & android.view.InputDevice.SOURCE_JOYSTICK) == android.view.InputDevice.SOURCE_JOYSTICK ||
                (source & android.view.InputDevice.SOURCE_GAMEPAD) == android.view.InputDevice.SOURCE_GAMEPAD) {

            if (event.getAction() == android.view.MotionEvent.ACTION_MOVE) {
                // Left stick
                float leftX = event.getAxisValue(android.view.MotionEvent.AXIS_X);
                float leftY = event.getAxisValue(android.view.MotionEvent.AXIS_Y);

                // Right stick
                float rightX = event.getAxisValue(android.view.MotionEvent.AXIS_Z);
                float rightY = event.getAxisValue(android.view.MotionEvent.AXIS_RZ);

                // Triggers (L2/R2)
                float l2 = event.getAxisValue(android.view.MotionEvent.AXIS_LTRIGGER);
                float r2 = event.getAxisValue(android.view.MotionEvent.AXIS_RTRIGGER);

                // If LTRIGGER/RTRIGGER is 0, try BRAKE/GAS (some controllers use these)
                if (l2 == 0)
                    l2 = event.getAxisValue(android.view.MotionEvent.AXIS_BRAKE);
                if (r2 == 0)
                    r2 = event.getAxisValue(android.view.MotionEvent.AXIS_GAS);

                // D-pad via HAT axes (PS5 DualSense uses this)
                float hatX = event.getAxisValue(android.view.MotionEvent.AXIS_HAT_X);
                float hatY = event.getAxisValue(android.view.MotionEvent.AXIS_HAT_Y);

                // D-pad Left/Right for volume control
                if (hatX != lastHatX) {
                    if (hatX < -0.5f) {
                        volumeDown();
                        Log.i(TAG, "D-pad LEFT: Volume Down");
                    } else if (hatX > 0.5f) {
                        volumeUp();
                        Log.i(TAG, "D-pad RIGHT: Volume Up");
                    }
                    lastHatX = hatX;
                }
                lastHatY = hatY;

                // Log.i(TAG, "GAMEPAD AXIS: LX=" + leftX + " LY=" + leftY + " RX=" + rightX + "
                // RY=" + rightY + " L2=" + l2 + " R2=" + r2);

                // Call Rust native method
                onGamepadAxis(leftX, leftY, rightX, rightY, l2, r2);

                return true;
            }
        }
        return super.dispatchGenericMotionEvent(event);
    }
}
