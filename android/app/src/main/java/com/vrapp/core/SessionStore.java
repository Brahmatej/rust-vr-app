package com.vrapp.core;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.Environment;
import android.util.Log;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

/**
 * THE persistence layer. Every piece of state that must outlive the process goes
 * through here — nothing else in the app touches SharedPreferences directly.
 *
 * It covers three things today:
 *   1. The tab session: per-tab URL, title and aspect-ratio index, plus the active
 *      tab. Serialized as one versioned blob so the format can evolve.
 *   2. Authentication: the Gecko profile directory (cookies, localStorage, logins)
 *      lives at a stable path we own, and the session-bearing files are mirrored to
 *      /sdcard so a REINSTALL does not sign the user out.
 *   3. Generic typed slots (getInt/putInt/getFloat/putFloat) so the settings that
 *      are still held in Rust — head-tracking mode, projection mode, zoom, stereo
 *      mode — can be folded in later without inventing a second mechanism.
 */
public final class SessionStore {
    private static final String TAG      = "VRAppJava";
    private static final String PREFS    = "vr_session";
    private static final int    SCHEMA   = 2;

    /** Unit separator: cannot occur in a URL or title. */
    private static final String SEP = "\u001F";

    private static final String K_SCHEMA  = "schema";
    private static final String K_TABS    = "tabs";
    private static final String K_ACTIVE  = "active_tab";

    /** One persisted tab. */
    public static final class TabRecord {
        public final String url;
        public final String title;
        public final int    aspect;
        public TabRecord(String url, String title, int aspect) {
            this.url = url; this.title = title; this.aspect = aspect;
        }
    }

    private final Context ctx;

    public SessionStore(Context ctx) { this.ctx = ctx; }

    private SharedPreferences prefs() {
        return ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    // ── Tab session ────────────────────────────────────────────────────────────

    /**
     * Records are stored one per line as `aspect  url  title`, with unit
     * separators so a URL or title can never break the format.
     */
    public List<TabRecord> loadTabs() {
        List<TabRecord> out = new ArrayList<>();
        SharedPreferences p = prefs();
        if (p.getInt(K_SCHEMA, 0) != SCHEMA) return out;   // unknown/older format: start fresh
        String blob = p.getString(K_TABS, "");
        if (blob.isEmpty()) return out;
        for (String line : blob.split("\n")) {
            if (line.trim().isEmpty()) continue;
            String[] f = line.split(SEP, -1);
            if (f.length < 3) continue;
            int aspect = 0;
            try { aspect = Integer.parseInt(f[0]); } catch (Exception ignored) {}
            String url = f[1].trim();
            if (url.isEmpty() || url.equals("about:blank")) continue;
            out.add(new TabRecord(url, f[2], aspect));
        }
        return out;
    }

    public int loadActiveTab() { return prefs().getInt(K_ACTIVE, 0); }

    public void saveTabs(List<TabRecord> tabs, int activeIndex) {
        StringBuilder sb = new StringBuilder();
        for (TabRecord t : tabs) {
            sb.append(t.aspect).append(SEP)
              .append(t.url == null ? "" : t.url.replace('\n', ' ')).append(SEP)
              .append(t.title == null ? "" : t.title.replace('\n', ' ')).append('\n');
        }
        prefs().edit()
            .putInt(K_SCHEMA, SCHEMA)
            .putString(K_TABS, sb.toString())
            .putInt(K_ACTIVE, activeIndex)
            .apply();
    }

    // ── Generic slots (for folding in the remaining settings later) ────────────

    public int   getInt(String key, int def)     { return prefs().getInt("v_" + key, def); }
    public void  putInt(String key, int value)   { prefs().edit().putInt("v_" + key, value).apply(); }
    public float getFloat(String key, float def) { return prefs().getFloat("v_" + key, def); }
    public void  putFloat(String key, float v)   { prefs().edit().putFloat("v_" + key, v).apply(); }

    // ── Authentication / cookies ──────────────────────────────────────────────

    /**
     * The Gecko profile. GeckoRuntime is started with `-profile <this>`, so cookies,
     * localStorage and saved logins persist across process death instead of landing
     * somewhere ephemeral.
     */
    public static File geckoProfileDir(Context ctx) {
        return new File(ctx.getDataDir(), "gecko_profile");
    }

    /**
     * Files that actually carry a signed-in session. The full profile is tens of MB
     * of caches, and the device runs near-full, so only these are mirrored.
     */
    private static final String[] AUTH_FILES = {
        "cookies.sqlite", "cookies.sqlite-wal", "cookies.sqlite-shm",
        "key4.db", "cert9.db", "logins.json",
        "webappsstore.sqlite", "webappsstore.sqlite-wal", "webappsstore.sqlite-shm",
        "places.sqlite", "prefs.js",
    };

    private File authBackupDir() {
        return new File(Environment.getExternalStorageDirectory(), "vrapp/gecko_backup");
    }

    /** Call before the runtime starts. No-op when a live profile already exists. */
    public void restoreAuth() {
        try {
            File profile = geckoProfileDir(ctx);
            if (new File(profile, "cookies.sqlite").exists()) {
                Log.i(TAG, "SessionStore: profile present, restore skipped");
                return;
            }
            File backup = authBackupDir();
            if (!backup.exists()) {
                Log.i(TAG, "SessionStore: no auth backup to restore");
                return;
            }
            if (!profile.exists() && !profile.mkdirs()) {
                Log.w(TAG, "SessionStore: cannot create " + profile);
                return;
            }
            int n = 0;
            for (String name : AUTH_FILES) {
                File src = new File(backup, name);
                if (src.isFile()) { copy(src, new File(profile, name)); n++; }
            }
            Log.i(TAG, "SessionStore: restored " + n + " auth files from " + backup);
        } catch (Exception e) {
            Log.w(TAG, "SessionStore.restoreAuth failed: " + e.getMessage());
        }
    }

    /** Call at onPause/onStop/onDestroy, after tab state has been flushed. */
    public void backupAuth() {
        try {
            File profile = geckoProfileDir(ctx);
            if (!profile.exists()) return;
            File backup = authBackupDir();
            if (!backup.exists() && !backup.mkdirs()) return;
            int n = 0;
            for (String name : AUTH_FILES) {
                File src = new File(profile, name);
                if (src.isFile()) { copy(src, new File(backup, name)); n++; }
            }
            if (n > 0) Log.i(TAG, "SessionStore: backed up " + n + " auth files to " + backup);
        } catch (Exception e) {
            Log.w(TAG, "SessionStore.backupAuth failed: " + e.getMessage());
        }
    }

    private static void copy(File src, File dst) throws IOException {
        File parent = dst.getParentFile();
        if (parent != null && !parent.exists()) parent.mkdirs();
        try (FileInputStream in = new FileInputStream(src);
             FileOutputStream out = new FileOutputStream(dst)) {
            byte[] buf = new byte[65536];
            int r;
            while ((r = in.read(buf)) > 0) out.write(buf, 0, r);
        }
    }
}
