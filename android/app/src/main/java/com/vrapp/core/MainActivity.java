package com.vrapp.core;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.res.Configuration;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Rect;
import android.media.AudioManager;
import android.media.MediaMetadataRetriever;
import android.media.MediaPlayer;
import android.media.ThumbnailUtils;
import android.net.Uri;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.PowerManager;
import android.speech.RecognitionListener;
import android.speech.SpeechRecognizer;
import android.util.Log;
import android.util.Size;
import android.view.Display;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Window;
import android.view.WindowManager;
import android.webkit.CookieManager;
import androidx.core.view.InputDeviceCompat;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ThreadFactory;

/* JADX INFO: loaded from: classes.dex */
public class MainActivity extends NativeActivity {
    private static final int MAX_WIDTH = 854;
    private static final int PICK_VIDEO_REQUEST = 1001;
    private static final String TAG = "VRAppJava";
    private byte[] frameBuffer;
    private Thread frameThread;
    private GamepadOverlay gamepadOverlay;
    private GeckoViewManager geckoViewManager;
    private MediaPlayer mediaPlayer;
    private int[] pixelBuffer;
    private MediaMetadataRetriever retriever;
    private SpeechRecognizer speechRecognizer;
    private PowerManager.WakeLock wakeLock;
    private volatile int frameWidth = 640;
    private volatile int frameHeight = 360;
    private volatile boolean hasVideo = false;
    private volatile boolean isRunning = false;
    private final Object lock = new Object();
    private Uri currentVideoUri = null;
    private ParcelFileDescriptor currentPfd = null;
    private boolean webViewReady = false;
    private boolean geckoReady = false;
    private int activeEngine = 0;
    private float displayHz = 60.0f;
    private boolean overlayAdded = false;
    private float lastHatX = 0.0f;
    private float lastHatY = 0.0f;
    private long lastVolumeChangeTime = 0;
    private int cachedGamepadDeviceId = -1;
    private final ExecutorService thumbPool = Executors.newFixedThreadPool(Math.max(2, Math.min(4, Runtime.getRuntime().availableProcessors())), new ThreadFactory() { // from class: com.vrapp.core.MainActivity$$ExternalSyntheticLambda1
        @Override // java.util.concurrent.ThreadFactory
        public final Thread newThread(Runnable runnable) {
            return MainActivity.lambda$new$3(runnable);
        }
    });
    private final Set<String> thumbInFlight = Collections.synchronizedSet(new HashSet());

    public native void onDisplayRotation(int i);

    public native void onDpadAxis(float f, float f2);

    public native void onGamepadAxis(float f, float f2, float f3, float f4, float f5, float f6);

    public native void onGamepadButton(int i, boolean z);

    public native void onThumbnail(String str, int i, int i2, byte[] bArr);

    public native void onVideoFdReady(int i);

    public native void onVideoPicked(String str);

    public native void onVoiceError(int i);

    public native void onVoiceResult(String str);

    public native void onWebFrame(int i, int i2, byte[] bArr);

    public void webViewIgNavigate(boolean z) {
    }

    public float getDisplayRefreshRate() {
        return this.displayHz;
    }

    static {
        System.loadLibrary("vr_core");
    }

    @Override // android.app.NativeActivity, android.app.Activity
    protected void onCreate(Bundle bundle) {
        super.onCreate(bundle);
        PowerManager.WakeLock wakeLockNewWakeLock = ((PowerManager) getSystemService("power")).newWakeLock(268435466, "vrapp:wakelock");
        this.wakeLock = wakeLockNewWakeLock;
        wakeLockNewWakeLock.acquire();
        Log.i(TAG, "MainActivity created - wake lock acquired");
        try {
            Window window = getWindow();
            WindowManager.LayoutParams attributes = window.getAttributes();
            Display defaultDisplay = getWindowManager().getDefaultDisplay();
            Display.Mode mode = defaultDisplay.getMode();
            Display.Mode[] supportedModes = defaultDisplay.getSupportedModes();
            float fAbs = Math.abs(mode.getRefreshRate() - 60.0f);
            Display.Mode mode2 = mode;
            for (Display.Mode mode3 : supportedModes) {
                if (mode3.getPhysicalWidth() == mode.getPhysicalWidth() && mode3.getPhysicalHeight() == mode.getPhysicalHeight()) {
                    float fAbs2 = Math.abs(mode3.getRefreshRate() - 60.0f);
                    if (fAbs2 < fAbs) {
                        mode2 = mode3;
                        fAbs = fAbs2;
                    }
                }
            }
            attributes.preferredDisplayModeId = mode2.getModeId();
            attributes.preferredRefreshRate = mode2.getRefreshRate();
            window.setAttributes(attributes);
            this.displayHz = mode2.getRefreshRate();
            Log.i(TAG, "Selected display mode " + mode2.getPhysicalWidth() + "x" + mode2.getPhysicalHeight() + " @ " + mode2.getRefreshRate() + " Hz (target 90)");
        } catch (Exception e) {
            Log.w(TAG, "Refresh-rate selection failed: " + e.getMessage());
        }
        requestAllFilesAccess();
        restoreWebData();
        this.activeEngine = 1;
    }

    private void requestAllFilesAccess() {
        try {
            if (Environment.isExternalStorageManager()) {
                Log.i(TAG, "All files access already granted");
                return;
            }
            Log.i(TAG, "Requesting MANAGE_EXTERNAL_STORAGE (All files access)");
            Intent intent = new Intent("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION");
            intent.setData(Uri.parse("package:" + getPackageName()));
            startActivity(intent);
        } catch (Exception e) {
            Log.w(TAG, "requestAllFilesAccess failed: " + e);
        }
    }

    public void checkVolumeButtons(boolean z, boolean z2) {
        long jCurrentTimeMillis = System.currentTimeMillis();
        if (jCurrentTimeMillis - this.lastVolumeChangeTime < 200) {
            return;
        }
        if (z) {
            volumeDown();
            this.lastVolumeChangeTime = jCurrentTimeMillis;
            Log.i(TAG, "Volume DOWN via native D-pad");
        }
        if (z2) {
            volumeUp();
            this.lastVolumeChangeTime = jCurrentTimeMillis;
            Log.i(TAG, "Volume UP via native D-pad");
        }
    }

    public void pollDpadForVolume() {
        if (this.cachedGamepadDeviceId == -1) {
            for (int i : InputDevice.getDeviceIds()) {
                InputDevice device = InputDevice.getDevice(i);
                if (device != null && (device.getSources() & 1025) == 1025) {
                    this.cachedGamepadDeviceId = i;
                    Log.i(TAG, "Found gamepad: " + device.getName() + " id=" + i);
                    return;
                }
            }
        }
    }

    public void volumeUp() {
        AudioManager audioManager = (AudioManager) getSystemService("audio");
        int streamVolume = audioManager.getStreamVolume(3);
        int streamMaxVolume = audioManager.getStreamMaxVolume(3);
        if (streamVolume < streamMaxVolume) {
            audioManager.setStreamVolume(3, streamVolume + 1, 1);
        }
        Log.i(TAG, "Volume Up: " + (streamVolume + 1) + "/" + streamMaxVolume);
    }

    public void volumeDown() {
        AudioManager audioManager = (AudioManager) getSystemService("audio");
        int streamVolume = audioManager.getStreamVolume(3);
        int streamMaxVolume = audioManager.getStreamMaxVolume(3);
        if (streamVolume > 0) {
            audioManager.setStreamVolume(3, streamVolume - 1, 1);
        }
        Log.i(TAG, "Volume Down: " + (streamVolume - 1) + "/" + streamMaxVolume);
    }

    public int getVolume() {
        return ((AudioManager) getSystemService("audio")).getStreamVolume(3);
    }

    public int getMaxVolume() {
        return ((AudioManager) getSystemService("audio")).getStreamMaxVolume(3);
    }

    public void launchVideoPicker() {
        Log.i(TAG, "Launching Video Picker (Google Photos)...");
        Intent intent = new Intent("android.intent.action.GET_CONTENT");
        intent.setType("video/*");
        intent.addCategory("android.intent.category.OPENABLE");
        intent.putExtra("android.intent.extra.LOCAL_ONLY", false);
        startActivityForResult(Intent.createChooser(intent, "Select Video"), 1001);
    }

    @Override // android.app.Activity
    protected void onActivityResult(int i, int i2, Intent intent) {
        super.onActivityResult(i, i2, intent);
        Log.i(TAG, "onActivityResult: req=" + i + " res=" + i2);
        if (i != 1001 || i2 != -1 || intent == null || intent.getData() == null) {
            return;
        }
        Uri data = intent.getData();
        try {
            getContentResolver().takePersistableUriPermission(data, 1);
        } catch (SecurityException e) {
            Log.w(TAG, "Failed to persist permission: " + e);
        }
        Log.i(TAG, "Selected Video URI: " + data);
        this.currentVideoUri = data;
        int videoFd = getVideoFd();
        if (videoFd >= 0) {
            Log.i(TAG, "Got file descriptor: " + videoFd);
            onVideoFdReady(videoFd);
        }
        startVideo(data);
        onVideoPicked(data.toString());
    }

    public int getVideoFd() {
        if (this.currentVideoUri == null) {
            return -1;
        }
        try {
            ParcelFileDescriptor parcelFileDescriptor = this.currentPfd;
            if (parcelFileDescriptor != null) {
                try {
                    parcelFileDescriptor.close();
                } catch (Exception unused) {
                }
            }
            ParcelFileDescriptor parcelFileDescriptorOpenFileDescriptor = getContentResolver().openFileDescriptor(this.currentVideoUri, "r");
            this.currentPfd = parcelFileDescriptorOpenFileDescriptor;
            if (parcelFileDescriptorOpenFileDescriptor != null) {
                return parcelFileDescriptorOpenFileDescriptor.detachFd();
            }
        } catch (FileNotFoundException e) {
            Log.e(TAG, "Failed to open file: " + e);
        }
        return -1;
    }

    private void startVideo(Uri uri) {
        stopVideo();
        try {
            MediaPlayer mediaPlayer = new MediaPlayer();
            this.mediaPlayer = mediaPlayer;
            mediaPlayer.setDataSource(this, uri);
            this.mediaPlayer.setOnPreparedListener(new MediaPlayer.OnPreparedListener() { // from class: com.vrapp.core.MainActivity.1
                @Override // android.media.MediaPlayer.OnPreparedListener
                public void onPrepared(MediaPlayer mediaPlayer2) {
                    Log.i(MainActivity.TAG, "Audio ready");
                    mediaPlayer2.start();
                    mediaPlayer2.setLooping(true);
                }
            });
            this.mediaPlayer.prepareAsync();
        } catch (Exception e) {
            Log.e(TAG, "Audio failed: " + e);
        }
        try {
            MediaMetadataRetriever mediaMetadataRetriever = new MediaMetadataRetriever();
            this.retriever = mediaMetadataRetriever;
            mediaMetadataRetriever.setDataSource(this, uri);
            String strExtractMetadata = this.retriever.extractMetadata(18);
            String strExtractMetadata2 = this.retriever.extractMetadata(19);
            int i = strExtractMetadata != null ? Integer.parseInt(strExtractMetadata) : 640;
            int i2 = strExtractMetadata2 != null ? Integer.parseInt(strExtractMetadata2) : 360;
            if (i > MAX_WIDTH) {
                i2 = (int) (i2 * (854.0f / i));
                i = MAX_WIDTH;
            }
            this.frameWidth = i;
            this.frameHeight = i2;
            int i3 = this.frameWidth * this.frameHeight;
            this.pixelBuffer = new int[i3];
            this.frameBuffer = new byte[i3 * 4];
            Log.i(TAG, "Video: " + this.frameWidth + "x" + this.frameHeight);
            this.hasVideo = true;
            this.isRunning = true;
            Thread thread = new Thread(new Runnable() { // from class: com.vrapp.core.MainActivity.2
                @Override // java.lang.Runnable
                public void run() {
                    MainActivity.this.extractFrames();
                }
            }, "FrameExtractor");
            this.frameThread = thread;
            thread.start();
        } catch (Exception e2) {
            Log.e(TAG, "Retriever failed: " + e2);
            this.hasVideo = false;
        }
    }

    /* JADX INFO: Access modifiers changed from: private */
    public void extractFrames() {
        MediaPlayer mediaPlayer;
        Bitmap bitmap;
        Bitmap bitmapCreateBitmap = null;
        while (this.isRunning && (mediaPlayer = this.mediaPlayer) != null && this.retriever != null) {
            try {
                if (!mediaPlayer.isPlaying()) {
                    Thread.sleep(50L);
                } else {
                    Bitmap frameAtTime = this.retriever.getFrameAtTime(((long) this.mediaPlayer.getCurrentPosition()) * 1000, 3);
                    if (frameAtTime != null) {
                        int i = 0;
                        if (frameAtTime.getWidth() == this.frameWidth && frameAtTime.getHeight() == this.frameHeight) {
                            bitmap = frameAtTime;
                        } else {
                            if (bitmapCreateBitmap == null || bitmapCreateBitmap.getWidth() != this.frameWidth) {
                                if (bitmapCreateBitmap != null) {
                                    bitmapCreateBitmap.recycle();
                                }
                                bitmapCreateBitmap = Bitmap.createBitmap(this.frameWidth, this.frameHeight, Bitmap.Config.ARGB_8888);
                            }
                            new Canvas(bitmapCreateBitmap).drawBitmap(frameAtTime, new Rect(0, 0, frameAtTime.getWidth(), frameAtTime.getHeight()), new Rect(0, 0, this.frameWidth, this.frameHeight), (Paint) null);
                            frameAtTime.recycle();
                            bitmap = bitmapCreateBitmap;
                        }
                        synchronized (this.lock) {
                            bitmap.getPixels(this.pixelBuffer, 0, this.frameWidth, 0, 0, this.frameWidth, this.frameHeight);
                            while (true) {
                                int[] iArr = this.pixelBuffer;
                                if (i >= iArr.length) {
                                    break;
                                }
                                int i2 = iArr[i];
                                int i3 = i * 4;
                                byte[] bArr = this.frameBuffer;
                                bArr[i3] = (byte) ((i2 >> 16) & 255);
                                bArr[i3 + 1] = (byte) ((i2 >> 8) & 255);
                                bArr[i3 + 2] = (byte) (i2 & 255);
                                bArr[i3 + 3] = -1;
                                i++;
                            }
                        }
                        if (bitmap != bitmapCreateBitmap) {
                            bitmap.recycle();
                        }
                    }
                    Thread.sleep(66L);
                }
            } catch (Exception e) {
                Log.e(TAG, "Frame error: " + e.getMessage());
                try {
                    Thread.sleep(100L);
                } catch (Exception unused) {
                }
            }
        }
        if (bitmapCreateBitmap != null) {
            bitmapCreateBitmap.recycle();
        }
    }

    private void stopVideo() {
        this.isRunning = false;
        this.hasVideo = false;
        Thread thread = this.frameThread;
        if (thread != null) {
            try {
                thread.join(500L);
            } catch (Exception unused) {
            }
            this.frameThread = null;
        }
        MediaPlayer mediaPlayer = this.mediaPlayer;
        if (mediaPlayer != null) {
            try {
                mediaPlayer.release();
            } catch (Exception unused2) {
            }
            this.mediaPlayer = null;
        }
        MediaMetadataRetriever mediaMetadataRetriever = this.retriever;
        if (mediaMetadataRetriever != null) {
            try {
                mediaMetadataRetriever.release();
            } catch (Exception unused3) {
            }
            this.retriever = null;
        }
    }

    public void startAudioFromPath(String str) {
        Log.i(TAG, "startAudioFromPath: " + str);
        MediaPlayer mediaPlayer = this.mediaPlayer;
        if (mediaPlayer != null) {
            try {
                mediaPlayer.release();
            } catch (Exception unused) {
            }
            this.mediaPlayer = null;
        }
        try {
            MediaPlayer mediaPlayer2 = new MediaPlayer();
            this.mediaPlayer = mediaPlayer2;
            mediaPlayer2.setDataSource(str);
            this.mediaPlayer.setOnErrorListener(new MediaPlayer.OnErrorListener() { // from class: com.vrapp.core.MainActivity.3
                @Override // android.media.MediaPlayer.OnErrorListener
                public boolean onError(MediaPlayer mediaPlayer3, int i, int i2) {
                    Log.e(MainActivity.TAG, "MediaPlayer ERROR: what=" + i + " extra=" + i2);
                    if (i2 != -1010) {
                        return true;
                    }
                    Log.e(MainActivity.TAG, "UNSUPPORTED AUDIO FORMAT - Try converting to AAC");
                    return true;
                }
            });
            this.mediaPlayer.setOnInfoListener(new MediaPlayer.OnInfoListener() { // from class: com.vrapp.core.MainActivity.4
                @Override // android.media.MediaPlayer.OnInfoListener
                public boolean onInfo(MediaPlayer mediaPlayer3, int i, int i2) {
                    Log.i(MainActivity.TAG, "MediaPlayer INFO: what=" + i + " extra=" + i2);
                    return false;
                }
            });
            this.mediaPlayer.setOnPreparedListener(new MediaPlayer.OnPreparedListener() { // from class: com.vrapp.core.MainActivity.5
                @Override // android.media.MediaPlayer.OnPreparedListener
                public void onPrepared(MediaPlayer mediaPlayer3) {
                    Log.i(MainActivity.TAG, "Audio ready from path - Duration: " + mediaPlayer3.getDuration() + "ms");
                    mediaPlayer3.start();
                    mediaPlayer3.setLooping(true);
                }
            });
            this.mediaPlayer.prepareAsync();
        } catch (Exception e) {
            Log.e(TAG, "startAudioFromPath failed: " + e);
        }
    }

    public void pauseAudio() {
        MediaPlayer mediaPlayer = this.mediaPlayer;
        if (mediaPlayer == null || !mediaPlayer.isPlaying()) {
            return;
        }
        try {
            this.mediaPlayer.pause();
            Log.i(TAG, "Audio paused");
        } catch (Exception e) {
            Log.e(TAG, "pauseAudio failed: " + e);
        }
    }

    public void resumeAudio() {
        MediaPlayer mediaPlayer = this.mediaPlayer;
        if (mediaPlayer == null || mediaPlayer.isPlaying()) {
            return;
        }
        try {
            this.mediaPlayer.start();
            Log.i(TAG, "Audio resumed");
        } catch (Exception e) {
            Log.e(TAG, "resumeAudio failed: " + e);
        }
    }

    public void seekAudio(int i) {
        MediaPlayer mediaPlayer = this.mediaPlayer;
        if (mediaPlayer != null) {
            try {
                mediaPlayer.seekTo(i);
                Log.i(TAG, "Audio seek to " + i + "ms");
            } catch (Exception e) {
                Log.e(TAG, "seekAudio failed: " + e);
            }
        }
    }

    public byte[] getVideoFrame() {
        byte[] bArr;
        if (!this.hasVideo || this.frameBuffer == null) {
            return null;
        }
        synchronized (this.lock) {
            bArr = this.frameBuffer;
        }
        return bArr;
    }

    public int getVideoWidth() {
        return this.frameWidth;
    }

    public int getVideoHeight() {
        return this.frameHeight;
    }

    @Override // android.app.NativeActivity, android.app.Activity, android.view.Window.Callback
    public void onWindowFocusChanged(boolean z) {
        super.onWindowFocusChanged(z);
        if (z) {
            addGamepadOverlay();
            GamepadOverlay gamepadOverlay = this.gamepadOverlay;
            if (gamepadOverlay != null) {
                gamepadOverlay.requestFocus();
            }
            reportDisplayRotation();
        }
    }

    @Override // android.app.NativeActivity, android.app.Activity, android.content.ComponentCallbacks
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        reportDisplayRotation();
    }

    private void reportDisplayRotation() {
        try {
            int rotation = getWindowManager().getDefaultDisplay().getRotation();
            Log.i(TAG, "Display rotation = " + rotation);
            onDisplayRotation(rotation);
        } catch (Exception e) {
            Log.w(TAG, "reportDisplayRotation failed: " + e);
        }
    }

    private void addGamepadOverlay() {
        if (this.overlayAdded) {
            return;
        }
        try {
            IBinder windowToken = getWindow().getDecorView().getWindowToken();
            if (windowToken == null) {
                Log.w(TAG, "addGamepadOverlay: window token not ready yet");
                return;
            }
            this.gamepadOverlay = new GamepadOverlay(this, this);
            WindowManager.LayoutParams layoutParams = new WindowManager.LayoutParams(-1, -1, 1000, 272, -3);
            layoutParams.token = windowToken;
            layoutParams.gravity = 8388659;
            ((WindowManager) getSystemService("window")).addView(this.gamepadOverlay, layoutParams);
            this.gamepadOverlay.requestFocus();
            this.overlayAdded = true;
            Log.i(TAG, "Gamepad overlay window added + focused");
        } catch (Exception e) {
            Log.e(TAG, "addGamepadOverlay failed: " + e);
        }
    }

    public void webViewLoadUrl(final String str) {
        new Handler(Looper.getMainLooper()).post(new Runnable() { // from class: com.vrapp.core.MainActivity$$ExternalSyntheticLambda2
            @Override // java.lang.Runnable
            public final void run() {
                MainActivity.this.lambda$webViewLoadUrl$0(str);
            }
        });
    }

    /* JADX INFO: Access modifiers changed from: private */
    public /* synthetic */ void lambda$webViewLoadUrl$0(String str) {
        ensureGeckoInit();
        if (this.geckoReady) {
            this.geckoViewManager.loadUrl(str);
        } else {
            Log.w(TAG, "webViewLoadUrl: Gecko not ready, dropping " + str);
        }
    }

    public void webViewGoBack() {
        if (this.geckoReady) {
            this.geckoViewManager.goBack();
        }
    }

    public void webViewGoForward() {
        if (this.geckoReady) {
            this.geckoViewManager.goForward();
        }
    }

    public void webViewReload() {
        if (this.geckoReady) {
            this.geckoViewManager.reload();
        }
    }

    public void webViewScroll(float f, float f2, float f3, float f4) {
        if (this.geckoReady) {
            this.geckoViewManager.scroll(f, f2, f3, f4);
        }
    }

    public void webViewResize(int i, int i2) {
        if (this.geckoReady) {
            this.geckoViewManager.resize(i, i2);
        }
    }

    public void webViewTap(float f, float f2) {
        if (this.geckoReady) {
            this.geckoViewManager.tap(f, f2);
        }
    }

    public void webViewTypeText(String str) {
        if (this.geckoReady) {
            this.geckoViewManager.typeText(str);
        }
    }

    public void webViewBackspace() {
        if (this.geckoReady) {
            this.geckoViewManager.backspace();
        }
    }

    public void webViewEnter() {
        if (this.geckoReady) {
            this.geckoViewManager.submitEnter();
        }
    }

    public void webViewNewTab() {
        ensureGeckoInit();
        if (this.geckoReady) {
            this.geckoViewManager.newTab("https://www.google.com");
        }
    }

    public void webViewSwitchTab(int i) {
        if (this.geckoReady) {
            this.geckoViewManager.switchTab(i);
        }
    }

    public void webViewCloseTab() {
        if (this.geckoReady) {
            this.geckoViewManager.closeTab();
        }
    }

    public void setBrowserActive(boolean z) {
        GeckoViewManager geckoViewManager = this.geckoViewManager;
        if (geckoViewManager != null) {
            geckoViewManager.setActive(z);
        }
    }

    public void setBrowserEngine(int i) {
        new Handler(Looper.getMainLooper()).post(new Runnable() { // from class: com.vrapp.core.MainActivity$$ExternalSyntheticLambda3
            @Override // java.lang.Runnable
            public final void run() {
                MainActivity.this.lambda$setBrowserEngine$1();
            }
        });
    }

    /* JADX INFO: Access modifiers changed from: private */
    public /* synthetic */ void lambda$setBrowserEngine$1() {
        this.activeEngine = 1;
        ensureGeckoInit();
        GeckoViewManager geckoViewManager = this.geckoViewManager;
        if (geckoViewManager != null) {
            geckoViewManager.setActive(true);
        }
        Log.i(TAG, "Browser engine -> Firefox (Gecko)");
    }

    private void ensureGeckoInit() {
        if (this.geckoViewManager != null) {
            return;
        }
        try {
            GeckoViewManager geckoViewManager = new GeckoViewManager(this);
            this.geckoViewManager = geckoViewManager;
            geckoViewManager.init();
            this.geckoReady = true;
            Log.i(TAG, "GeckoViewManager ready");
        } catch (Throwable th) {
            Log.e(TAG, "Gecko init failed: " + th);
            this.geckoViewManager = null;
            this.geckoReady = false;
        }
    }

    public void startVoiceSearch() {
        Log.i(TAG, "startVoiceSearch called");
        new Handler(Looper.getMainLooper()).post(new Runnable() { // from class: com.vrapp.core.MainActivity$$ExternalSyntheticLambda4
            @Override // java.lang.Runnable
            public final void run() {
                MainActivity.this.lambda$startVoiceSearch$2();
            }
        });
    }

    /* JADX INFO: Access modifiers changed from: private */
    public /* synthetic */ void lambda$startVoiceSearch$2() {
        SpeechRecognizer speechRecognizer = this.speechRecognizer;
        if (speechRecognizer != null) {
            speechRecognizer.destroy();
            this.speechRecognizer = null;
        }
        if (!SpeechRecognizer.isRecognitionAvailable(this)) {
            Log.e(TAG, "SpeechRecognizer not available");
            onVoiceError(5);
            return;
        }
        SpeechRecognizer speechRecognizerCreateSpeechRecognizer = SpeechRecognizer.createSpeechRecognizer(this);
        this.speechRecognizer = speechRecognizerCreateSpeechRecognizer;
        speechRecognizerCreateSpeechRecognizer.setRecognitionListener(new RecognitionListener() { // from class: com.vrapp.core.MainActivity.6
            @Override // android.speech.RecognitionListener
            public void onBufferReceived(byte[] bArr) {
            }

            @Override // android.speech.RecognitionListener
            public void onEvent(int i, Bundle bundle) {
            }

            @Override // android.speech.RecognitionListener
            public void onPartialResults(Bundle bundle) {
            }

            @Override // android.speech.RecognitionListener
            public void onRmsChanged(float f) {
            }

            @Override // android.speech.RecognitionListener
            public void onReadyForSpeech(Bundle bundle) {
                Log.i(MainActivity.TAG, "Voice: ready");
            }

            @Override // android.speech.RecognitionListener
            public void onBeginningOfSpeech() {
                Log.i(MainActivity.TAG, "Voice: started");
            }

            @Override // android.speech.RecognitionListener
            public void onEndOfSpeech() {
                Log.i(MainActivity.TAG, "Voice: end");
            }

            @Override // android.speech.RecognitionListener
            public void onError(int i) {
                Log.e(MainActivity.TAG, "Voice error: " + i);
                MainActivity.this.onVoiceError(i);
            }

            @Override // android.speech.RecognitionListener
            public void onResults(Bundle bundle) {
                ArrayList<String> stringArrayList = bundle.getStringArrayList("results_recognition");
                if (stringArrayList != null && !stringArrayList.isEmpty()) {
                    String str = stringArrayList.get(0);
                    Log.i(MainActivity.TAG, "Voice result: " + str);
                    MainActivity.this.onVoiceResult(str);
                    return;
                }
                MainActivity.this.onVoiceError(7);
            }
        });
        Intent intent = new Intent("android.speech.action.RECOGNIZE_SPEECH");
        intent.putExtra("android.speech.extra.LANGUAGE_MODEL", "web_search");
        intent.putExtra("android.speech.extra.PARTIAL_RESULTS", false);
        intent.putExtra("android.speech.extra.MAX_RESULTS", 1);
        this.speechRecognizer.startListening(intent);
    }

    @Override // android.app.NativeActivity, android.app.Activity
    protected void onPause() {
        GeckoViewManager geckoViewManager = this.geckoViewManager;
        if (geckoViewManager != null) {
            geckoViewManager.saveTabState();
        }
        backupWebData();
        super.onPause();
    }

    @Override // android.app.NativeActivity, android.app.Activity
    protected void onStop() {
        GeckoViewManager geckoViewManager = this.geckoViewManager;
        if (geckoViewManager != null) {
            geckoViewManager.saveTabState();
        }
        backupWebData();
        super.onStop();
    }

    @Override // android.app.NativeActivity, android.app.Activity
    protected void onDestroy() {
        backupWebData();
        stopVideo();
        if (this.overlayAdded && this.gamepadOverlay != null) {
            try {
                ((WindowManager) getSystemService("window")).removeView(this.gamepadOverlay);
            } catch (Exception e) {
                Log.w(TAG, "removeView overlay: " + e);
            }
            this.overlayAdded = false;
        }
        GeckoViewManager geckoViewManager = this.geckoViewManager;
        if (geckoViewManager != null) {
            geckoViewManager.destroy();
        }
        SpeechRecognizer speechRecognizer = this.speechRecognizer;
        if (speechRecognizer != null) {
            speechRecognizer.destroy();
            this.speechRecognizer = null;
        }
        PowerManager.WakeLock wakeLock = this.wakeLock;
        if (wakeLock != null && wakeLock.isHeld()) {
            this.wakeLock.release();
        }
        super.onDestroy();
    }

    private File webDataDir() {
        return new File(getDataDir(), "app_webview");
    }

    private File webBackupDir() {
        return new File(Environment.getExternalStorageDirectory(), "vrapp/webview_backup");
    }

    private void restoreWebData() {
        try {
            File fileWebDataDir = webDataDir();
            File fileWebBackupDir = webBackupDir();
            File file = new File(fileWebDataDir, "Default/Cookies");
            File file2 = new File(fileWebDataDir, "Cookies");
            if (!file.exists() && !file2.exists()) {
                if (!fileWebBackupDir.exists()) {
                    Log.i(TAG, "WebData restore: no backup found");
                    return;
                } else {
                    copyRecursive(fileWebBackupDir, fileWebDataDir);
                    Log.i(TAG, "WebData restored from " + fileWebBackupDir.getAbsolutePath());
                    return;
                }
            }
            Log.i(TAG, "WebData restore skipped (existing session present)");
        } catch (Exception e) {
            Log.w(TAG, "restoreWebData failed: " + e.getMessage());
        }
    }

    private void backupWebData() {
        try {
            CookieManager.getInstance().flush();
        } catch (Exception unused) {
        }
        try {
            File fileWebDataDir = webDataDir();
            if (fileWebDataDir.exists()) {
                File fileWebBackupDir = webBackupDir();
                File file = new File(fileWebBackupDir.getParentFile(), "webview_backup.tmp");
                deleteRecursive(file);
                copyRecursive(fileWebDataDir, file);
                deleteRecursive(fileWebBackupDir);
                if (!file.renameTo(fileWebBackupDir)) {
                    copyRecursive(file, fileWebBackupDir);
                    deleteRecursive(file);
                }
                Log.i(TAG, "WebData backed up to " + fileWebBackupDir.getAbsolutePath());
            }
        } catch (Exception e) {
            Log.w(TAG, "backupWebData failed: " + e.getMessage());
        }
    }

    private void copyRecursive(File file, File file2) throws IOException {
        if (file.isDirectory()) {
            if (!file2.exists()) {
                file2.mkdirs();
            }
            String[] list = file.list();
            if (list != null) {
                for (String str : list) {
                    copyRecursive(new File(file, str), new File(file2, str));
                }
                return;
            }
            return;
        }
        File parentFile = file2.getParentFile();
        if (parentFile != null && !parentFile.exists()) {
            parentFile.mkdirs();
        }
        FileInputStream fileInputStream = new FileInputStream(file);
        try {
            FileOutputStream fileOutputStream = new FileOutputStream(file2);
            try {
                byte[] bArr = new byte[65536];
                while (true) {
                    int i = fileInputStream.read(bArr);
                    if (i <= 0) {
                        fileOutputStream.close();
                        fileInputStream.close();
                        return;
                    }
                    fileOutputStream.write(bArr, 0, i);
                }
            } finally {
            }
        } catch (Throwable th) {
            try {
                fileInputStream.close();
            } catch (Throwable th2) {
                th.addSuppressed(th2);
            }
            throw th;
        }
    }

    private void deleteRecursive(File file) {
        String[] list;
        if (file == null || !file.exists()) {
            return;
        }
        if (file.isDirectory() && (list = file.list()) != null) {
            for (String str : list) {
                deleteRecursive(new File(file, str));
            }
        }
        file.delete();
    }

    static /* synthetic */ Thread lambda$new$3(Runnable runnable) {
        Thread thread = new Thread(runnable, "thumb-gen");
        thread.setPriority(1);
        return thread;
    }

    public void requestThumbnail(final String str, final int i, final int i2) {
        if (str == null || !this.thumbInFlight.add(str)) {
            return;
        }
        this.thumbPool.execute(new Runnable() { // from class: com.vrapp.core.MainActivity$$ExternalSyntheticLambda0
            @Override // java.lang.Runnable
            public final void run() {
                MainActivity.this.lambda$requestThumbnail$4(str, i, i2);
            }
        });
    }

    /* JADX INFO: Access modifiers changed from: private */
    public /* synthetic */ void lambda$requestThumbnail$4(String str, int i, int i2) {
        MediaMetadataRetriever mediaMetadataRetriever = new MediaMetadataRetriever();
        try {
            try {
                mediaMetadataRetriever.setDataSource(str);
                Bitmap scaledFrameAtTime = mediaMetadataRetriever.getScaledFrameAtTime(1000000L, 2, i, i2);
                if (scaledFrameAtTime == null) {
                    scaledFrameAtTime = mediaMetadataRetriever.getScaledFrameAtTime(1000000L, 3, i, i2);
                }
                if (scaledFrameAtTime == null) {
                    scaledFrameAtTime = mediaMetadataRetriever.getScaledFrameAtTime(0L, 2, i, i2);
                }
                if (scaledFrameAtTime == null) {
                    Bitmap frameAtTime = mediaMetadataRetriever.getFrameAtTime(1000000L, 2);
                    if (frameAtTime == null) {
                        frameAtTime = mediaMetadataRetriever.getFrameAtTime(-1L);
                    }
                    if (frameAtTime != null && frameAtTime != (scaledFrameAtTime = Bitmap.createScaledBitmap(frameAtTime, i, i2, true))) {
                        frameAtTime.recycle();
                    }
                }
                if (scaledFrameAtTime == null) {
                    try {
                        scaledFrameAtTime = ThumbnailUtils.createVideoThumbnail(new File(str), new Size(i, i2), null);
                    } catch (Throwable unused) {
                    }
                }
                if (scaledFrameAtTime != null) {
                    if (scaledFrameAtTime.getConfig() != Bitmap.Config.ARGB_8888) {
                        Bitmap bitmapCopy = scaledFrameAtTime.copy(Bitmap.Config.ARGB_8888, false);
                        scaledFrameAtTime.recycle();
                        scaledFrameAtTime = bitmapCopy;
                    }
                    int width = scaledFrameAtTime.getWidth();
                    int height = scaledFrameAtTime.getHeight();
                    ByteBuffer byteBufferAllocate = ByteBuffer.allocate(width * height * 4);
                    scaledFrameAtTime.copyPixelsToBuffer(byteBufferAllocate);
                    scaledFrameAtTime.recycle();
                    onThumbnail(str, width, height, byteBufferAllocate.array());
                } else {
                    onThumbnail(str, 0, 0, new byte[0]);
                }
            } catch (Exception e) {
                Log.w(TAG, "thumbnail failed for " + str + ": " + e.getMessage());
                onThumbnail(str, 0, 0, new byte[0]);
            }
            try {
                mediaMetadataRetriever.release();
            } catch (Exception unused2) {
            }
            this.thumbInFlight.remove(str);
        } catch (Throwable th) {
            try {
                mediaMetadataRetriever.release();
            } catch (Exception unused3) {
            }
            this.thumbInFlight.remove(str);
            throw th;
        }
    }

    @Override // android.app.Activity, android.view.Window.Callback
    public boolean dispatchKeyEvent(KeyEvent keyEvent) {
        int source = keyEvent.getSource();
        if ((source & 1025) == 1025 || (source & InputDeviceCompat.SOURCE_JOYSTICK) == 16777232) {
            int keyCode = keyEvent.getKeyCode();
            boolean z = keyEvent.getAction() == 0;
            Log.i(TAG, "GAMEPAD KEY: code=" + keyCode + " pressed=" + z);
            if (z) {
                if (keyCode == 21) {
                    volumeDown();
                    Log.i(TAG, "D-pad LEFT KeyEvent: Volume Down");
                } else if (keyCode == 22) {
                    volumeUp();
                    Log.i(TAG, "D-pad RIGHT KeyEvent: Volume Up");
                }
            }
            onGamepadButton(keyCode, z);
            return true;
        }
        return super.dispatchKeyEvent(keyEvent);
    }

    @Override // android.app.Activity, android.view.Window.Callback
    public boolean dispatchGenericMotionEvent(MotionEvent motionEvent) {
        Log.i(TAG, "dispatchGenericMotionEvent CALLED! source=" + motionEvent.getSource());
        int source = motionEvent.getSource();
        if (((source & InputDeviceCompat.SOURCE_JOYSTICK) == 16777232 || (source & 1025) == 1025) && motionEvent.getAction() == 2) {
            float axisValue = motionEvent.getAxisValue(0);
            float axisValue2 = motionEvent.getAxisValue(1);
            float axisValue3 = motionEvent.getAxisValue(11);
            float axisValue4 = motionEvent.getAxisValue(14);
            float axisValue5 = motionEvent.getAxisValue(17);
            float axisValue6 = motionEvent.getAxisValue(18);
            if (axisValue5 == 0.0f) {
                axisValue5 = motionEvent.getAxisValue(23);
            }
            if (axisValue6 == 0.0f) {
                axisValue6 = motionEvent.getAxisValue(22);
            }
            float f = axisValue6;
            float axisValue7 = motionEvent.getAxisValue(15);
            float axisValue8 = motionEvent.getAxisValue(16);
            float f2 = this.lastHatX;
            boolean z = (axisValue7 == f2 && axisValue8 == this.lastHatY) ? false : true;
            if (axisValue7 != f2) {
                if (axisValue7 < -0.5f) {
                    volumeDown();
                    Log.i(TAG, "D-pad LEFT: Volume Down");
                } else if (axisValue7 > 0.5f) {
                    volumeUp();
                    Log.i(TAG, "D-pad RIGHT: Volume Up");
                }
                this.lastHatX = axisValue7;
            }
            if (axisValue8 != this.lastHatY) {
                if (axisValue8 < -0.5f) {
                    Log.i(TAG, "D-pad UP");
                } else if (axisValue8 > 0.5f) {
                    Log.i(TAG, "D-pad DOWN");
                }
                this.lastHatY = axisValue8;
            }
            if (z) {
                onDpadAxis(axisValue7, axisValue8);
                Log.i(TAG, "HAT axis sent to Rust: x=" + axisValue7 + " y=" + axisValue8);
            }
            onGamepadAxis(axisValue, axisValue2, axisValue3, axisValue4, axisValue5, f);
            return true;
        }
        return super.dispatchGenericMotionEvent(motionEvent);
    }
}
