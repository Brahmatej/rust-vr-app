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

    @Override
    public boolean dispatchKeyEvent(android.view.KeyEvent event) {
        // Log key events to debug controller
        Log.i(TAG, "JAVA KEY: " + event.toString());
        return super.dispatchKeyEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(android.view.MotionEvent ev) {
        // Log motion events (sticks, triggers)
        Log.i(TAG, "JAVA MOTION: " + ev.toString());
        return super.dispatchGenericMotionEvent(ev);
    }
}
