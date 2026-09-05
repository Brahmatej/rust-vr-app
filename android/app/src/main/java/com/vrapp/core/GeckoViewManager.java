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

import org.mozilla.geckoview.ContentBlocking;
import org.mozilla.geckoview.GeckoDisplay;
import org.mozilla.geckoview.GeckoResult;
import org.mozilla.geckoview.GeckoRuntime;
import org.mozilla.geckoview.GeckoSession;
import org.mozilla.geckoview.GeckoSessionSettings;

import java.nio.ByteBuffer;

/**
 * Firefox (GeckoView) engine rendering into ImageReader Surfaces.
 *
 * Multi-tab model: every tab is a GeckoSession on the SAME GeckoRuntime, so all
 * tabs share one PERSISTENT profile → shared cookies / logins that survive
 * process death. CRUCIALLY each tab owns its OWN GeckoDisplay + ImageReader/
 * Surface, acquired once at creation and never swapped. Switching tabs only
 * changes which tab pushes frames to the renderer.
 *
 * (An earlier design shared one display and re-acquired it on every switch — that
 * tears down/rebuilds WebRender's compositor mid-frame and segfaults the Gecko
 * thread with "webrender error 3". Per-tab displays avoid all of that.)
 *
 * Each tab also carries its OWN aspect ratio (viewport shape), cycled with D-pad
 * right from the VR side; the Rust renderer derives the browser plane's shape
 * from the pushed frame size, so the two always agree.
 */
public class GeckoViewManager {
    private static final String TAG      = "VRAppJava";
    private static final int    WEB_W    = 1920;
    private static final int    WEB_H    = 1080;
    /**
     * Tab cap. Each tab owns a GeckoSession *and* an ImageReader, so the cost is
     * real: at 1920x1080 RGBX with 2 buffers that is ~16.6 MB of graphics memory
     * per tab plus the session itself. 6 tabs ≈ 100 MB of readers, which the
     * headset handles; going much past that risks the compositor. The buffer
     * count was dropped 3 -> 2 (acquireLatestImage only ever needs double
     * buffering) precisely to pay for the two extra tabs.
     */
    private static final int    MAX_TABS = 6;
    private static final int    READER_BUFFERS = 2;
    private static final String HOME_URL = "https://www.google.com";

    /** Width of the cached per-tab preview handed to the VR tab overview. */
    private static final int    THUMB_W  = 240;

    /** Per-tab viewport shapes, cycled with D-pad right. */
    private static final int[][] ASPECTS = {
        {1920, 1080},   // 16:9  widescreen
        {1440, 1080},   // 4:3   classic
        {2160,  926},   // 21:9  ultrawide
        {1200, 1200},   // 1:1   square
        {1080, 1920},   // 9:16  tall / phone layout
    };
    private static final String[] ASPECT_LABELS = { "16:9", "4:3", "21:9", "1:1", "9:16" };

    private final Context context;
    private final Handler mainHandler;
    private final SessionStore store;

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
        String  title = "";
        int     progress = 100;
        boolean loaded = false;
        boolean inFullscreen = false;
        boolean textFocused = false;
        int     aspect = 0;
        int     w = WEB_W, h = WEB_H;
    }

    private final java.util.List<Tab> tabs = new java.util.ArrayList<>();
    private int activeTab = 0;

    /**
     * Per-tab preview bitmaps (12-byte header w/h/seq + RGBA), indexed by tab
     * index. Written on the ImageReader thread, read by the VR render thread via
     * {@link #getTabThumb(int)} — an atomic array so neither side ever walks the
     * `tabs` list off the main thread (GeckoView is UI-thread affine).
     */
    private final java.util.concurrent.atomic.AtomicReferenceArray<byte[]> thumbs =
        new java.util.concurrent.atomic.AtomicReferenceArray<>(MAX_TABS);
    private int thumbTick = 0;

    /**
     * Snapshot of {@link #getTabInfo()} published from the MAIN thread.
     *
     * The render thread used to call getTabInfo() directly, walking Gecko-owned
     * state off the UI thread. Now the UI thread republishes this string whenever
     * the model changes (and on a slow tick for progress/title), and the render
     * thread only ever reads this one volatile reference.
     */
    private volatile String tabInfoSnapshot = "0\t0\t100\t0\t" + MAX_TABS + "\n";

    /** Last-resort feedback for the VR chrome: e.g. "Tab limit reached (6)". */
    private volatile String notice = "";
    private volatile long   noticeAtMs = 0;

    private byte[] frameBuf;
    private volatile boolean running = false;
    private volatile boolean active  = false; // only push frames when this engine is selected

    public GeckoViewManager(Context context) {
        this.context     = context;
        this.mainHandler = new Handler(Looper.getMainLooper());
        this.store       = new SessionStore(context);
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

        java.io.File profile = SessionStore.geckoProfileDir(context);
        if (!profile.exists() && !profile.mkdirs()) {
            Log.w(TAG, "Could not create Gecko profile dir " + profile);
        }

        // Cookies and site storage must actually stick, otherwise a Google login
        // evaporates on every restart. ACCEPT_ALL + ETP off is the most permissive
        // (and most login-compatible) configuration.
        ContentBlocking.Settings cb = new ContentBlocking.Settings.Builder()
            .cookieBehavior(ContentBlocking.CookieBehavior.ACCEPT_ALL)
            .antiTracking(ContentBlocking.AntiTracking.NONE)
            .enhancedTrackingProtectionLevel(ContentBlocking.EtpLevel.NONE)
            .cookiePurging(false)
            .build();

        org.mozilla.geckoview.GeckoRuntimeSettings settings =
            new org.mozilla.geckoview.GeckoRuntimeSettings.Builder()
                .screenSizeOverride(WEB_W, WEB_H)
                // -profile pins Gecko to a stable directory we control (and can back
                // up); without it the profile lives wherever Gecko decides and is not
                // guaranteed to survive a reinstall.
                .arguments(new String[]{ "-profile", profile.getAbsolutePath() })
                .contentBlocking(cb)
                .loginAutofillEnabled(true)
                .aboutConfigEnabled(true)
                .javaScriptEnabled(true)
                .build();
        try {
            runtime = GeckoRuntime.create(context, settings);
        } catch (Throwable t) {
            // Never let a profile/argument problem take the whole browser down —
            // fall back to a default runtime and log loudly.
            Log.e(TAG, "GeckoRuntime.create with -profile failed, retrying default: " + t);
            runtime = GeckoRuntime.create(context,
                new org.mozilla.geckoview.GeckoRuntimeSettings.Builder()
                    .screenSizeOverride(WEB_W, WEB_H)
                    .contentBlocking(cb)
                    .build());
        }
        imeView = new View(context);

        running = true;

        java.util.List<SessionStore.TabRecord> saved = store.loadTabs();
        if (saved.isEmpty()) {
            createTab(HOME_URL, null, 0);
        } else {
            for (SessionStore.TabRecord r : saved) {
                Tab t = createTab(r.url, null, r.aspect);
                t.title = r.title == null ? "" : r.title;
            }
        }
        activeTab = Math.max(0, Math.min(store.loadActiveTab(), tabs.size() - 1));
        activateTab(activeTab);
        mainHandler.post(infoTicker);

        Log.i(TAG, "GeckoViewManager initialised " + WEB_W + "x" + WEB_H
            + " with " + tabs.size() + " tab(s), active " + activeTab
            + ", profile " + profile.getAbsolutePath());
    }

    // ── Tabs ────────────────────────────────────────────────────────────────────

    private String sanitize(String url) {
        if (url == null || url.trim().isEmpty() || url.trim().equals("about:blank")) return HOME_URL;
        return url.trim();
    }

    private GeckoSessionSettings sessionSettings() {
        // Desktop UA + desktop viewport: the panel is 1920-wide-class, and Google's
        // sign-in flow is happiest with a plain desktop Firefox identity. GeckoView is
        // real Firefox, so no "unsupported browser" interstitial.
        return new GeckoSessionSettings.Builder()
            .usePrivateMode(false)
            .useTrackingProtection(false)
            .allowJavascript(true)
            .suspendMediaWhenInactive(false)
            .userAgentMode(GeckoSessionSettings.USER_AGENT_MODE_DESKTOP)
            .viewportMode(GeckoSessionSettings.VIEWPORT_MODE_DESKTOP)
            .build();
    }

    /**
     * Build a tab with its own session + display pipeline.
     *
     * `existing` is non-null only on the popup path (`onNewSession`), where Gecko
     * hands us an UNOPENED session that it opens itself — we must not open it.
     */
    private Tab createTab(String url, GeckoSession existing, int aspectIdx) {
        final Tab tab = new Tab();
        tab.url    = sanitize(url);
        tab.aspect = (aspectIdx >= 0 && aspectIdx < ASPECTS.length) ? aspectIdx : 0;
        tab.w = ASPECTS[tab.aspect][0];
        tab.h = ASPECTS[tab.aspect][1];

        // Own capture pipeline (RGBX_8888 = Gecko compositor format).
        tab.reader = ImageReader.newInstance(tab.w, tab.h, PixelFormat.RGBX_8888, READER_BUFFERS);
        tab.reader.setOnImageAvailableListener(r -> onImageAvailable(r, tab), readerHandler);

        tab.session = (existing != null) ? existing : new GeckoSession(sessionSettings());
        attachDelegates(tab);
        if (existing == null) {
            tab.session.open(runtime);
        } else {
            // Gecko opens and navigates it for us.
            tab.loaded = true;
        }

        // Own display, bound once to this tab's surface and kept for its lifetime.
        try {
            tab.display = tab.session.acquireDisplay();
            tab.display.surfaceChanged(
                new GeckoDisplay.SurfaceInfo.Builder(tab.reader.getSurface())
                    .size(tab.w, tab.h).build());
        } catch (Exception e) {
            // Popup sessions are not open yet at this point on some paths; retry once
            // the runtime has taken ownership.
            Log.w(TAG, "acquireDisplay deferred: " + e);
            mainHandler.postDelayed(() -> {
                try {
                    if (tab.session == null || tab.reader == null) return;
                    tab.display = tab.session.acquireDisplay();
                    tab.display.surfaceChanged(
                        new GeckoDisplay.SurfaceInfo.Builder(tab.reader.getSurface())
                            .size(tab.w, tab.h).build());
                    Log.i(TAG, "acquireDisplay (deferred) ok");
                } catch (Exception e2) {
                    Log.e(TAG, "acquireDisplay (deferred) failed: " + e2);
                }
            }, 300);
        }

        tabs.add(tab);
        return tab;
    }

    private void attachDelegates(final Tab tab) {
        tab.session.setContentDelegate(new GeckoSession.ContentDelegate() {
            @Override public void onFullScreen(GeckoSession sess, boolean fs) {
                tab.inFullscreen = fs;
                Log.i(TAG, "Gecko fullscreen video: " + fs);
            }
            @Override public void onTitleChange(GeckoSession sess, String title) {
                tab.title = (title != null) ? title : "";
            }
            @Override public void onCloseRequest(GeckoSession sess) {
                // window.close() from a popup: drop that tab.
                mainHandler.post(() -> closeTabObject(tab));
            }
        });
        tab.session.setProgressDelegate(new GeckoSession.ProgressDelegate() {
            @Override public void onPageStart(GeckoSession sess, String locUrl) {
                if (locUrl != null) tab.url = locUrl;
                tab.progress = 0;
            }
            @Override public void onPageStop(GeckoSession sess, boolean ok) {
                tab.progress = 100;
            }
            @Override public void onProgressChange(GeckoSession sess, int p) {
                tab.progress = p;
            }
        });
        tab.session.setNavigationDelegate(new GeckoSession.NavigationDelegate() {
            @Override public void onLocationChange(GeckoSession sess, String url,
                    java.util.List<GeckoSession.PermissionDelegate.ContentPermission> perms,
                    Boolean hasUserGesture) {
                if (url != null && !url.equals("about:blank")) tab.url = url;
            }
            /**
             * THE api for target="_blank" / window.open() / popups. Without it such
             * links silently do nothing. Returning a fresh (unopened) session here
             * makes Gecko drive it, and we register it as a new tab.
             */
            @Override public GeckoResult<GeckoSession> onNewSession(GeckoSession sess, String uri) {
                if (tabs.size() >= MAX_TABS) {
                    Log.i(TAG, "onNewSession: at MAX_TABS, loading in place -> " + uri);
                    if (uri != null) sess.loadUri(uri);
                    return GeckoResult.fromValue(null);
                }
                GeckoSession child = new GeckoSession(sessionSettings());
                Tab t = createTab(uri, child, tab.aspect);
                t.title = "New tab";
                activateTab(tabs.indexOf(t));
                Log.i(TAG, "onNewSession -> tab " + activeTab + " (" + uri + ")");
                return GeckoResult.fromValue(child);
            }
            @Override public GeckoResult<org.mozilla.geckoview.AllowOrDeny> onLoadRequest(
                    GeckoSession sess, GeckoSession.NavigationDelegate.LoadRequest req) {
                // Some target="_blank" navigations arrive here rather than through
                // onNewSession; open those in a real new tab instead of losing them.
                if (req != null && req.target == GeckoSession.NavigationDelegate.TARGET_WINDOW_NEW
                        && tabs.size() < MAX_TABS) {
                    final String uri = req.uri;
                    mainHandler.post(() -> {
                        Tab t = createTab(uri, null, tab.aspect);
                        activateTab(tabs.indexOf(t));
                        Log.i(TAG, "onLoadRequest TARGET_WINDOW_NEW -> tab " + activeTab);
                    });
                    return GeckoResult.deny();
                }
                return GeckoResult.allow();
            }
        });
        // Focused-text-field detection: Gecko tells us when an editable node takes or
        // loses focus, which is what auto-opens the VR keyboard.
        tab.session.getTextInput().setDelegate(new GeckoSession.TextInputDelegate() {
            @Override public void restartInput(GeckoSession sess, int reason) {
                if (reason == GeckoSession.TextInputDelegate.RESTART_REASON_FOCUS) {
                    tab.textFocused = true;
                } else if (reason == GeckoSession.TextInputDelegate.RESTART_REASON_BLUR) {
                    tab.textFocused = false;
                }
            }
            @Override public void showSoftInput(GeckoSession sess) { tab.textFocused = true; }
            @Override public void hideSoftInput(GeckoSession sess) { tab.textFocused = false; }
        });
    }

    /** Make a tab active: resume it, pause others, load lazily. No display juggling. */
    private void activateTab(int idx) {
        if (idx < 0 || idx >= tabs.size()) return;

        // Snapshot the OUTGOING tab first: `frameBuf` still holds its last frame,
        // and once it is backgrounded it will never produce another one.
        Tab prev = active();
        if (prev != null && activeTab != idx && frameBuf != null) {
            captureThumb(activeTab, prev.w, prev.h);
        }

        activeTab = idx;
        Tab tab = tabs.get(idx);

        // Exactly ONE tab may be active, focused and composited. Leaving the old
        // tab focused/active is what left its content painted underneath the new
        // one — two live surfaces racing into the same renderer texture.
        for (int i = 0; i < tabs.size(); i++) {
            Tab t = tabs.get(i);
            try { t.session.setActive(i == idx); }  catch (Exception e) {}
            try { t.session.setFocused(i == idx); } catch (Exception e) {}
        }
        try { tab.session.getTextInput().setView(imeView); } catch (Exception e) {}

        // Blank the renderer's page texture: the incoming tab paints nothing until
        // its first frame lands, and without this the previous tab's pixels stay on
        // screen underneath it.
        clearRendererFrame(tab.w, tab.h);

        if (!tab.loaded) {
            String want = sanitize(tab.url);
            Log.i(TAG, "Loading tab " + idx + " -> " + want);
            tab.session.loadUri(want);
            tab.loaded = true;
        }
        publishTabInfo();
        Log.i(TAG, "GECKO TAB active=" + activeTab + " of " + tabs.size());
    }

    /**
     * Push one opaque blank frame so the renderer stops showing the tab we just
     * left. Uses a small buffer: the Rust side scales whatever it is given.
     */
    private void clearRendererFrame(int w, int h) {
        if (!(context instanceof MainActivity) || !active) return;
        int cw = 64, ch = Math.max(1, 64 * h / Math.max(1, w));
        byte[] blank = new byte[cw * ch * 4];
        for (int i = 0; i < blank.length; i += 4) {
            blank[i] = 24; blank[i + 1] = 24; blank[i + 2] = 28; blank[i + 3] = (byte) 0xFF;
        }
        ((MainActivity) context).onWebFrame(cw, ch, blank);
    }

    public void newTab(String url) {
        mainHandler.post(() -> {
            if (tabs.size() >= MAX_TABS) {
                // Used to fail silently, which read as a dead button.
                notice("Tab limit reached (" + MAX_TABS + ") — close one first");
                Log.i(TAG, "newTab: at MAX_TABS");
                return;
            }
            Tab old = active();
            if (old != null && old.inFullscreen) old.session.exitFullScreen();
            createTab(url, null, old != null ? old.aspect : 0);
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

    /** Jump straight to a tab (tab-overview selection). */
    public void selectTab(int index) {
        mainHandler.post(() -> {
            if (index < 0 || index >= tabs.size() || index == activeTab) return;
            Tab old = active();
            if (old != null && old.inFullscreen) old.session.exitFullScreen();
            activateTab(index);
        });
    }

    public void closeTab() {
        mainHandler.post(() -> closeTabAtIndex(activeTab));
    }

    /** Close a specific tab (from the overview grid). */
    public void closeTabAt(int index) {
        mainHandler.post(() -> closeTabAtIndex(index));
    }

    private void closeTabObject(Tab t) {
        int i = tabs.indexOf(t);
        if (i >= 0) closeTabAtIndex(i);
    }

    private void closeTabAtIndex(int index) {
        if (index < 0 || index >= tabs.size()) return;
        if (tabs.size() <= 1) {
            // Last tab: never leave zero tabs (nothing would render). Reset it home.
            Tab t = tabs.get(0);
            t.session.loadUri(HOME_URL);
            t.url = HOME_URL;
            t.title = "";
            t.loaded = true;
            thumbs.set(0, null);
            notice("Last tab — reset to home");
            publishTabInfo();
            Log.i(TAG, "closeTab: last tab kept, reset to home");
            return;
        }
        Tab dead = tabs.remove(index);
        destroyTab(dead);
        shiftThumbsAfterRemoval(index);
        // Re-index: closing a tab BELOW the active one shifts it down by one;
        // closing the active one (or anything above) leaves the index alone, then
        // gets clamped into the shortened list.
        if (activeTab > index) activeTab--;
        if (activeTab >= tabs.size()) activeTab = tabs.size() - 1;
        // activateTab() early-returns when the index is unchanged in the caller's
        // eyes, so force the full re-activation path by re-running it here.
        int want = activeTab;
        activeTab = -1;
        activateTab(want);
        Log.i(TAG, "Closed Gecko tab " + index + "; " + tabs.size() + " left, active " + activeTab);
    }

    private void destroyTab(Tab t) {
        try { if (t.session != null && t.display != null) t.session.releaseDisplay(t.display); } catch (Exception e) {}
        try { if (t.session != null) t.session.close(); } catch (Exception e) {}
        if (t.reader != null) { try { t.reader.close(); } catch (Exception e) {} }
        t.display = null; t.session = null; t.reader = null;
    }

    public int getTabCount()  { return tabs.size(); }
    public int getActiveTab() { return activeTab; }

    // ── Per-tab aspect ratio ───────────────────────────────────────────────────

    /** Cycle the ACTIVE tab to the next viewport shape. */
    public void cycleAspect() {
        mainHandler.post(() -> {
            Tab t = active();
            if (t == null) return;
            t.aspect = (t.aspect + 1) % ASPECTS.length;
            applyAspect(t);
            Log.i(TAG, "Tab " + activeTab + " aspect -> " + ASPECT_LABELS[t.aspect]);
        });
    }

    private void applyAspect(Tab t) {
        resizeTab(t, ASPECTS[t.aspect][0], ASPECTS[t.aspect][1]);
    }

    /**
     * The render thread's view of the tab model. Returns the string most recently
     * published by the MAIN thread — it never touches Gecko state itself.
     */
    public String getTabInfo() { return tabInfoSnapshot; }

    /** Republish {@link #tabInfoSnapshot} + a heartbeat. MAIN THREAD ONLY. */
    private final Runnable infoTicker = new Runnable() {
        @Override public void run() {
            if (!running) return;
            publishTabInfo();
            mainHandler.postDelayed(this, 150);
        }
    };

    /** Show a short message in the VR browser chrome (e.g. the tab-cap refusal). */
    private void notice(String msg) {
        notice = msg == null ? "" : msg;
        noticeAtMs = SystemClock.uptimeMillis();
        publishTabInfo();
    }

    /**
     * Build the whole tab model for the VR UI, as one string. MAIN THREAD ONLY:
     *   line 0: activeIndex\tcount\tprogress\ttextFocused\tmaxTabs\tnotice
     *   line N: url\ttitle\taspectLabel\taspectIndex
     */
    private void publishTabInfo() {
        StringBuilder sb = new StringBuilder();
        Tab a = active();
        // Notices are transient: 4 s and they stop being reported.
        String note = (SystemClock.uptimeMillis() - noticeAtMs < 4000) ? notice : "";
        sb.append(activeTab).append('\t')
          .append(tabs.size()).append('\t')
          .append(a != null ? a.progress : 100).append('\t')
          .append(a != null && a.textFocused ? 1 : 0).append('\t')
          .append(MAX_TABS).append('\t')
          .append(note.replace('\t', ' ').replace('\n', ' ')).append('\n');
        for (Tab t : tabs) {
            String u = t.url == null ? "" : t.url;
            String ti = t.title == null ? "" : t.title;
            sb.append(u.replace('\t', ' ').replace('\n', ' ')).append('\t')
              .append(ti.replace('\t', ' ').replace('\n', ' ')).append('\t')
              .append(ASPECT_LABELS[t.aspect]).append('\t')
              .append(t.aspect).append('\n');
        }
        tabInfoSnapshot = sb.toString();
    }

    // ── Per-tab previews ───────────────────────────────────────────────────────

    /**
     * Cached preview of tab `index`: 12-byte big-endian header (width, height,
     * sequence) followed by tightly-packed RGBA, or null if that tab has not been
     * seen on screen yet. Safe to call from any thread.
     */
    public byte[] getTabThumb(int index) {
        if (index < 0 || index >= MAX_TABS) return null;
        return thumbs.get(index);
    }

    /**
     * Box-downscale the frame we just captured for the active tab into its preview
     * slot. Runs on the ImageReader thread (never the render thread) and only
     * every ~45 frames, so the cost is negligible and previews are at most a
     * second stale — which is what makes tab previews possible at all: only the
     * ACTIVE tab has a compositor surface, so the only way to show a preview of a
     * backgrounded tab is to have kept one from while it was in front.
     */
    private void captureThumb(int index, int srcW, int srcH) {
        if (index < 0 || index >= MAX_TABS || srcW <= 0 || srcH <= 0) return;
        int tw = Math.min(THUMB_W, srcW);
        int th = Math.max(1, (int) ((long) tw * srcH / srcW));
        byte[] out = new byte[12 + tw * th * 4];
        putInt(out, 0, tw);
        putInt(out, 4, th);
        putInt(out, 8, (int) (SystemClock.uptimeMillis() / 100));

        int srcRow = srcW * 4;
        for (int y = 0; y < th; y++) {
            int sy = y * srcH / th;
            int sBase = sy * srcRow;
            int dBase = 12 + y * tw * 4;
            for (int x = 0; x < tw; x++) {
                int s = sBase + (x * srcW / tw) * 4;
                int d = dBase + x * 4;
                out[d]     = frameBuf[s];
                out[d + 1] = frameBuf[s + 1];
                out[d + 2] = frameBuf[s + 2];
                out[d + 3] = (byte) 0xFF;   // RGBX: the 4th channel is padding
            }
        }
        thumbs.set(index, out);
    }

    private static void putInt(byte[] b, int off, int v) {
        b[off]     = (byte) (v >>> 24);
        b[off + 1] = (byte) (v >>> 16);
        b[off + 2] = (byte) (v >>> 8);
        b[off + 3] = (byte) v;
    }

    /** Keep preview slots aligned with tab indices after a tab is removed. */
    private void shiftThumbsAfterRemoval(int removed) {
        for (int i = removed; i < MAX_TABS - 1; i++) thumbs.set(i, thumbs.get(i + 1));
        thumbs.set(MAX_TABS - 1, null);
    }

    // ── Tab-state persistence (delegated to SessionStore) ─────────────────────

    public void saveTabState() {
        try {
            java.util.List<SessionStore.TabRecord> recs = new java.util.ArrayList<>();
            for (Tab t : tabs) {
                String u = (t.url != null && !t.url.isEmpty() && !t.url.equals("about:blank"))
                    ? t.url : HOME_URL;
                recs.add(new SessionStore.TabRecord(u, t.title, t.aspect));
                // Push any pending session data (cookies, form state) to disk.
                try { t.session.flushSessionState(); } catch (Exception e) {}
            }
            store.saveTabs(recs, activeTab);
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

            // Keep a downscaled preview of whatever is on screen, so that when this
            // tab is backgrounded (and loses its only compositor output) the tab
            // overview still has something real to show.
            if (++thumbTick % 45 == 0) captureThumb(activeTab, w, h);
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
            if (t != null) resizeTab(t, w, h);
        });
    }

    /** Rebuild one tab's capture surface at a new size (main thread only). */
    private void resizeTab(Tab t, int w, int h) {
        if (t == null || t.display == null) return;
        if (t.w == w && t.h == h) return;
        try {
            ImageReader old = t.reader;
            final Tab ft = t;
            t.reader = ImageReader.newInstance(w, h, PixelFormat.RGBX_8888, READER_BUFFERS);
            t.reader.setOnImageAvailableListener(r -> onImageAvailable(r, ft), readerHandler);
            t.display.surfaceChanged(
                new GeckoDisplay.SurfaceInfo.Builder(t.reader.getSurface())
                    .size(w, h).build());
            if (old != null) old.close();
            t.w = w; t.h = h;
            Log.i(TAG, "Gecko (tab " + tabs.indexOf(t) + ") resized to " + w + "x" + h);
        } catch (Exception e) {
            Log.e(TAG, "Gecko resize failed: " + e);
        }
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
