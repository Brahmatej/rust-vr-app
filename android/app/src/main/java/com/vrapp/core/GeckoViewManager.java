package com.vrapp.core;

import android.content.Context;
import android.graphics.PixelFormat;
import android.media.Image;
import android.media.ImageReader;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

import org.mozilla.geckoview.GeckoDisplay;
import org.mozilla.geckoview.GeckoRuntime;
import org.mozilla.geckoview.GeckoSession;

import java.nio.ByteBuffer;

/**
 * Firefox (GeckoView) engine rendering into ImageReader Surfaces.
 *
 * Multi-tab model: every tab is a GeckoSession on the SAME GeckoRuntime, so all
 * tabs share one profile → shared cookies / logins. CRUCIALLY each tab owns its
 * OWN GeckoDisplay + ImageReader/Surface, acquired once at creation and never
 * swapped. Switching tabs only changes which tab pushes frames to the renderer.
 *
 * (An earlier design shared one display and re-acquired it on every switch — that
 * tears down/rebuilds WebRender's compositor mid-frame and segfaults the Gecko
 * thread with "webrender error 3". Per-tab displays avoid all of that.)
 */
public class GeckoViewManager {
    private static final String TAG      = "VRAppJava";
    private static final int    WEB_W    = 1920;
    private static final int    WEB_H    = 1080;
    private static final int    MAX_TABS = 4;   // each tab = its own compositor (heavy)
    private static final String HOME_URL = "https://www.google.com";

    private final Context context;
    private final Handler mainHandler;

    private GeckoRuntime runtime;
    private HandlerThread readerThread;
    private Handler       readerHandler;
    private View          imeView;

    /** One tab = one session + its own display pipeline. */
    private final class Tab {
        GeckoSession session;
        GeckoDisplay display;
        ImageReader  reader;
        String  url;
        boolean loaded = false;
        boolean inFullscreen = false;
        int     w = WEB_W, h = WEB_H;
    }

    private final java.util.List<Tab> tabs = new java.util.ArrayList<>();
    private int activeTab = 0;

    private byte[] frameBuf;
    private volatile boolean running = false;
    private volatile boolean active  = false; // only push frames when this engine is selected

    public GeckoViewManager(Context context) {
        this.context     = context;
        this.mainHandler = new Handler(Looper.getMainLooper());
    }

    private Tab active() {
        return (activeTab >= 0 && activeTab < tabs.size()) ? tabs.get(activeTab) : null;
    }
    private GeckoSession activeSession() { Tab t = active(); return t != null ? t.session : null; }

    /** Must be called on the main (UI) thread. Boots the Gecko engine + tabs. */
    public void init() {
        frameBuf = new byte[WEB_W * WEB_H * 4];
        readerThread = new HandlerThread("GeckoImageReader");
        readerThread.start();
        readerHandler = new Handler(readerThread.getLooper());

        org.mozilla.geckoview.GeckoRuntimeSettings settings =
            new org.mozilla.geckoview.GeckoRuntimeSettings.Builder()
                .screenSizeOverride(WEB_W, WEB_H)
                .build();
        runtime = GeckoRuntime.create(context, settings);
        imeView = new View(context);

        running = true;

        java.util.List<String> saved = loadTabState();
        if (saved.isEmpty()) {
            createTab(HOME_URL);
        } else {
            for (String u : saved) createTab(u);
        }
        activeTab = Math.max(0, Math.min(loadActiveIndex(), tabs.size() - 1));
        activateTab(activeTab);

        Log.i(TAG, "GeckoViewManager initialised " + WEB_W + "x" + WEB_H
            + " with " + tabs.size() + " tab(s), active " + activeTab);
    }

    // ── Tabs ────────────────────────────────────────────────────────────────────

    private String sanitize(String url) {
        if (url == null || url.trim().isEmpty() || url.trim().equals("about:blank")) return HOME_URL;
        return url.trim();
    }

    /** Build a tab with its own session + display pipeline (no load yet). */
    private Tab createTab(String url) {
        final Tab tab = new Tab();
        tab.url = sanitize(url);

        // Own capture pipeline (RGBX_8888 = Gecko compositor format).
        tab.reader = ImageReader.newInstance(WEB_W, WEB_H, PixelFormat.RGBX_8888, 3);
        tab.reader.setOnImageAvailableListener(r -> onImageAvailable(r, tab), readerHandler);

        tab.session = new GeckoSession();
        tab.session.setContentDelegate(new GeckoSession.ContentDelegate() {
            @Override public void onFullScreen(GeckoSession sess, boolean fs) {
                tab.inFullscreen = fs;
                Log.i(TAG, "Gecko fullscreen video: " + fs);
            }
        });
        tab.session.setProgressDelegate(new GeckoSession.ProgressDelegate() {
            @Override public void onPageStart(GeckoSession sess, String locUrl) {
                if (locUrl != null) tab.url = locUrl;
            }
        });
        tab.session.open(runtime);

        // Own display, bound once to this tab's surface and kept for its lifetime.
        tab.display = tab.session.acquireDisplay();
        tab.display.surfaceChanged(
            new GeckoDisplay.SurfaceInfo.Builder(tab.reader.getSurface())
                .size(WEB_W, WEB_H).build());

        tabs.add(tab);
        return tab;
    }

    /** Make a tab active: resume it, pause others, load lazily. No display juggling. */
    private void activateTab(int idx) {
        if (idx < 0 || idx >= tabs.size()) return;
        activeTab = idx;
        Tab tab = tabs.get(idx);

        for (int i = 0; i < tabs.size(); i++) {
            try { tabs.get(i).session.setActive(i == idx); } catch (Exception e) {}
        }
        tab.session.getTextInput().setView(imeView);

        if (!tab.loaded) {
            String want = sanitize(tab.url);
            Log.i(TAG, "Loading tab " + idx + " -> " + want);
            tab.session.loadUri(want);
            tab.loaded = true;
        }
        Log.i(TAG, "GECKO TAB active=" + activeTab + " of " + tabs.size());
    }

    public void newTab(String url) {
        mainHandler.post(() -> {
            if (tabs.size() >= MAX_TABS) { Log.i(TAG, "newTab: at MAX_TABS"); return; }
            Tab old = active();
            if (old != null && old.inFullscreen) old.session.exitFullScreen();
            createTab(url);
            activateTab(tabs.size() - 1);
            Log.i(TAG, "Opened Gecko tab " + activeTab + " (" + tabs.size() + " total)");
        });
    }

    public void switchTab(int delta) {
        mainHandler.post(() -> {
            if (tabs.size() <= 1) return;
            Tab old = active();
            if (old != null && old.inFullscreen) old.session.exitFullScreen();
            int next = ((activeTab + delta) % tabs.size() + tabs.size()) % tabs.size();
            activateTab(next);
            Log.i(TAG, "Switched to Gecko tab " + activeTab);
        });
    }

    public void closeTab() {
        mainHandler.post(() -> {
            if (tabs.size() <= 1) { Log.i(TAG, "closeTab: last tab kept"); return; }
            Tab dead = tabs.remove(activeTab);
            destroyTab(dead);
            if (activeTab >= tabs.size()) activeTab = tabs.size() - 1;
            activateTab(activeTab);
            Log.i(TAG, "Closed Gecko tab; " + tabs.size() + " left, active " + activeTab);
        });
    }

    private void destroyTab(Tab t) {
        try { if (t.session != null && t.display != null) t.session.releaseDisplay(t.display); } catch (Exception e) {}
        try { if (t.session != null) t.session.close(); } catch (Exception e) {}
        if (t.reader != null) { try { t.reader.close(); } catch (Exception e) {} }
        t.display = null; t.session = null; t.reader = null;
    }

    public int getTabCount()  { return tabs.size(); }
    public int getActiveTab() { return activeTab; }

    // ── Tab-state persistence ──────────────────────────────────────────────────

    private android.content.SharedPreferences prefs() {
        return context.getSharedPreferences("vr_gecko_tabs", Context.MODE_PRIVATE);
    }
    private java.util.List<String> loadTabState() {
        java.util.List<String> out = new java.util.ArrayList<>();
        String joined = prefs().getString("tab_urls", "");
        if (!joined.isEmpty()) {
            for (String u : joined.split("\n")) {
                if (u == null) continue;
                String t = u.trim();
                if (!t.isEmpty() && !t.equals("about:blank")) out.add(t);
            }
        }
        return out;
    }
    private int loadActiveIndex() { return prefs().getInt("active_tab", 0); }

    public void saveTabState() {
        try {
            StringBuilder sb = new StringBuilder();
            for (Tab t : tabs) {
                String u = (t.url != null && !t.url.isEmpty() && !t.url.equals("about:blank")) ? t.url : HOME_URL;
                sb.append(u).append('\n');
            }
            prefs().edit()
                .putString("tab_urls", sb.toString())
                .putInt("active_tab", activeTab)
                .apply();
        } catch (Exception e) { Log.w(TAG, "saveTabState (gecko) failed: " + e); }
    }

    private void onImageAvailable(ImageReader reader, Tab tab) {
        Image image = null;
        try {
            image = reader.acquireLatestImage();
            if (image == null) return;
            // Only the active tab of the selected engine pushes frames.
            if (!running || !active || tab != active()) return;

            Image.Plane plane = image.getPlanes()[0];
            ByteBuffer buf = plane.getBuffer();
            int rowStride = plane.getRowStride();
            int w = image.getWidth();
            int h = image.getHeight();
            int dstRow = w * 4;
            int needed = dstRow * h;
            if (frameBuf == null || frameBuf.length < needed) frameBuf = new byte[needed];

            if (rowStride == dstRow) {
                buf.get(frameBuf, 0, needed);
            } else {
                for (int row = 0; row < h; row++) {
                    buf.position(row * rowStride);
                    buf.get(frameBuf, row * dstRow, dstRow);
                }
            }

            if (context instanceof MainActivity) {
                ((MainActivity) context).onWebFrame(w, h, frameBuf);
            }
        } catch (Exception e) {
            Log.e(TAG, "Gecko onImageAvailable error: " + e.getMessage());
        } finally {
            if (image != null) image.close();
        }
    }

    /** Toggle whether this engine pushes frames to the renderer. */
    public void setActive(boolean a) { this.active = a; }

    private InputConnection ic() {
        GeckoSession s = activeSession();
        if (s == null) return null;
        return s.getTextInput().onCreateInputConnection(new EditorInfo());
    }

    // ── Controls (called from Rust via JNI bridge), all operate on the active tab ─

    public void loadUrl(String url) {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null) return;
            t.session.loadUri(url);
            t.url = url;
            t.loaded = true;
        });
    }
    public void goBack() {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null) return;
            if (t.inFullscreen) { t.session.exitFullScreen(); return; }
            t.session.goBack();
        });
    }
    public void goForward() {
        mainHandler.post(() -> { GeckoSession s = activeSession(); if (s != null) s.goForward(); });
    }
    public void reload() {
        mainHandler.post(() -> { GeckoSession s = activeSession(); if (s != null) s.reload(); });
    }

    private boolean dragging = false;
    private float dragX, dragY;
    private long   dragDownT;
    private final Runnable endDrag = new Runnable() {
        @Override public void run() {
            if (activeSession() == null || !dragging) return;
            sendTouch(android.view.MotionEvent.ACTION_UP, dragX, dragY);
            dragging = false;
        }
    };

    private void sendTouch(int action, float x, float y) {
        GeckoSession s = activeSession();
        if (s == null) return;
        android.view.MotionEvent e = android.view.MotionEvent.obtain(
            dragDownT, android.os.SystemClock.uptimeMillis(), action, x, y, 0);
        e.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
        s.getPanZoomController().onTouchEvent(e);
        e.recycle();
    }

    public void scroll(float dx, float dy, float cx, float cy) {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null) return;
            if (!dragging) {
                dragX = cx * t.w; dragY = cy * t.h;
                dragDownT = android.os.SystemClock.uptimeMillis();
                sendTouch(android.view.MotionEvent.ACTION_DOWN, dragX, dragY);
                dragging = true;
            }
            dragX -= dx; dragY -= dy;
            if (dragX < 4 || dragX > t.w - 4 || dragY < 4 || dragY > t.h - 4) {
                sendTouch(android.view.MotionEvent.ACTION_UP,
                    Math.max(4, Math.min(t.w - 4, dragX)),
                    Math.max(4, Math.min(t.h - 4, dragY)));
                dragging = false;
            } else {
                sendTouch(android.view.MotionEvent.ACTION_MOVE, dragX, dragY);
            }
            mainHandler.removeCallbacks(endDrag);
            mainHandler.postDelayed(endDrag, 130);
        });
    }

    public void tap(float xNorm, float yNorm) {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null) return;
            long ts = android.os.SystemClock.uptimeMillis();
            float px = xNorm * t.w, py = yNorm * t.h;
            android.view.MotionEvent d = android.view.MotionEvent.obtain(
                ts, ts, android.view.MotionEvent.ACTION_DOWN, px, py, 0);
            android.view.MotionEvent u = android.view.MotionEvent.obtain(
                ts, ts + 60, android.view.MotionEvent.ACTION_UP, px, py, 0);
            d.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
            u.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
            t.session.getPanZoomController().onTouchEvent(d);
            t.session.getPanZoomController().onTouchEvent(u);
            d.recycle();
            u.recycle();
        });
    }

    public void typeText(String text) {
        mainHandler.post(() -> {
            InputConnection c = ic();
            if (c != null) c.commitText(text, 1);
            else Log.w(TAG, "Gecko typeText: no input connection (focus a field first)");
        });
    }
    public void backspace() {
        mainHandler.post(() -> {
            InputConnection c = ic();
            if (c != null) c.deleteSurroundingText(1, 0);
        });
    }
    public void resize(int w, int h) {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null || t.display == null) return;
            try {
                ImageReader old = t.reader;
                t.reader = ImageReader.newInstance(w, h, PixelFormat.RGBX_8888, 3);
                final Tab ft = t;
                t.reader.setOnImageAvailableListener(r -> onImageAvailable(r, ft), readerHandler);
                t.display.surfaceChanged(
                    new GeckoDisplay.SurfaceInfo.Builder(t.reader.getSurface())
                        .size(w, h).build());
                if (old != null) old.close();
                t.w = w; t.h = h;
                Log.i(TAG, "Gecko (tab " + activeTab + ") resized to " + w + "x" + h);
            } catch (Exception e) {
                Log.e(TAG, "Gecko resize failed: " + e);
            }
        });
    }

    public void submitEnter() {
        mainHandler.post(() -> {
            GeckoSession s = activeSession();
            if (s == null) return;
            long t = SystemClock.uptimeMillis();
            s.getTextInput().onKeyDown(KeyEvent.KEYCODE_ENTER,
                new KeyEvent(t, t, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER, 0));
            s.getTextInput().onKeyUp(KeyEvent.KEYCODE_ENTER,
                new KeyEvent(t, t, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER, 0));
        });
    }

    public void destroy() {
        running = false;
        active  = false;
        saveTabState();
        mainHandler.post(() -> {
            for (Tab t : tabs) destroyTab(t);
            tabs.clear();
        });
        if (readerThread != null) { readerThread.quitSafely(); readerThread = null; }
    }
}
