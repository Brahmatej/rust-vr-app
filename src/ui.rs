use egui::{Context, Visuals, Style, Rounding, Color32, Margin, Stroke, FontId, FontFamily};
use std::time::Instant;
use std::path::PathBuf;

// ── VR tunable parameters ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct VrParams {
    pub lens_radius:        f32,
    pub lens_center_offset: f32,
    pub content_scale:      f32,
    #[allow(dead_code)]
    pub target_scale:       f32,   // lerp target for smooth zoom
    pub gyro_enabled:       bool,
    // Web mode
    pub web_mode:           bool,
    pub browser_engine:     i32,        // 0 = Chromium (unused), 1 = Firefox/Gecko
    // Stereoscopic video layout: 0 = mono, 1 = SBS, 2 = over-under.
    pub stereo_mode:        u8,
    /// Screen geometry: 0 = flat curved screen, 1 = 180 dome, 2 = 360 dome,
    /// 3 = vertical (portrait/tall) panel. Cycled with D-pad up.
    pub projection_mode:    u8,
}

impl Default for VrParams {
    fn default() -> Self {
        Self {
            lens_radius:        1.0,
            lens_center_offset: 0.0,
            content_scale:      1.0,
            target_scale:       1.0,
            gyro_enabled:       true,
            web_mode:           false,
            browser_engine:     1,
            stereo_mode:        0,
            projection_mode:    0,
        }
    }
}

pub const STEREO_MODES: u8 = 3;
pub const PROJECTION_MODES: u8 = 4;

pub fn projection_label(mode: u8) -> &'static str {
    match mode {
        1 => "180° Dome",
        2 => "360° Dome",
        3 => "Vertical",
        _ => "Flat Screen",
    }
}

pub fn stereo_label(mode: u8) -> &'static str {
    match mode { 1 => "3D · Side-by-Side", 2 => "3D · Over-Under", _ => "2D · Mono" }
}

pub enum MenuState { Main, LensSettings, WebBrowser }

// ── Focus: the single source of truth for who owns input ─────────────────────
//
// Exactly one surface has focus at a time. Every panel's visibility is DERIVED
// from this (see `VrUi::dock_visible` etc.) — there are deliberately no parallel
// `visible: bool` fields that could disagree with it, because that disagreement
// is what used to make openers fail to toggle and bindings fight each other.
//
// Input dispatch in lib.rs matches on this enum, so every binding conflict has a
// mechanical answer: D-pad right cycles stereo in `Video`, cycles the focused
// tab's aspect ratio in `Browser`.
//
// NOTE: focus is about INPUT, not about what is on the screen plane. Whether the
// browser page or a video is being drawn is `VrParams::web_mode`; you can have the
// dock focused while the page is still visible behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    /// Nothing open: media transport + projection/stereo bindings.
    Video,
    /// The M3 dock.
    Dock,
    /// Media Center (file browser carousel).
    MediaCenter,
    /// On-screen VR keyboard.
    Keyboard,
    /// Browser page has the controller: stick cursor, click, scroll, tabs.
    Browser,
    /// Browser tab grid.
    TabOverview,
}

// ── Intents: one-shot commands, queued and drained ───────────────────────────
//
// Replaces the old `*_flag: bool` fields that were set in one place and manually
// cleared in another (miss a clear → it fires every frame; miss a read → the
// event is swallowed). An intent is pushed once, drained exactly once by lib.rs,
// and cannot leak into the next frame.
#[derive(Clone, Debug)]
pub enum Intent {
    Recenter,
    ExitVr,
    PlayFile(PathBuf),
    TogglePlayPause,
    /// Relative seek in microseconds.
    Seek(i64),
    // ── Browser ──
    SetEngine(i32),
    Navigate(String),
    BrowserBack,
    BrowserForward,
    BrowserReload,
    NewTab,
    CloseTabAt(usize),
    SelectTab(usize),
    /// Relative tab move (+1 next, -1 previous), wrapping.
    SwitchTab(i32),
    CycleAspect,
    /// Normalized (0..1) page coordinates.
    Tap(f32, f32),
    /// dx, dy, focus x, focus y (the latter two normalized).
    Scroll(f32, f32, f32, f32),
    TypeText(String),
    SubmitEnter,
}

// ── Material 3 Expressive colour roles (dark scheme, baseline violet) ─────────
// Google's M3 dark tonal palette: surfaces are low-chroma neutrals, primary is the
// tone-80 accent with tone-20 "on" colours, so accented controls read as light-on-
// dark rather than the flat blue-on-white the old dock used.
pub const M3_SURFACE_CONTAINER: Color32       = Color32::from_rgba_premultiplied(28, 27, 31, 242);
pub const M3_SURFACE_HIGH: Color32            = Color32::from_rgb(54, 52, 59);
pub const M3_ON_SURFACE: Color32              = Color32::from_rgb(230, 224, 233);
pub const M3_PRIMARY: Color32                 = Color32::from_rgb(208, 188, 255);
pub const M3_ON_PRIMARY: Color32              = Color32::from_rgb(56, 30, 114);
pub const M3_SECONDARY_CONTAINER: Color32     = Color32::from_rgb(74, 68, 88);
pub const M3_ON_SECONDARY_CONTAINER: Color32  = Color32::from_rgb(232, 222, 248);
pub const M3_ERROR: Color32                   = Color32::from_rgb(242, 184, 181);
pub const M3_ON_ERROR: Color32                = Color32::from_rgb(96, 20, 16);
pub const M3_ERROR_SOFT: Color32              = Color32::from_rgb(242, 184, 181);

// ── Center dock ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum DockItem {
    Recenter,
    Files,
    Firefox,
    Tabs,
    Keyboard,
    Stereo3D,
    SeekBack,
    PlayPause,
    SeekFwd,
    Settings,
    Exit,
}

/// Trimmed to what actually does something. Removed: Gyro (no visible effect),
/// Web (duplicate of Firefox - both call activate_browser(1)), and New/Close Tab
/// (multi-tab UI isn't built yet, so they were dead controls taking up space).
pub const DOCK_ITEMS: [DockItem; 11] = [
    DockItem::Recenter,
    DockItem::Files,
    DockItem::Firefox,
    DockItem::Tabs,
    DockItem::Keyboard,
    DockItem::Stereo3D,
    DockItem::SeekBack,
    DockItem::PlayPause,
    DockItem::SeekFwd,
    DockItem::Settings,
    DockItem::Exit,
];

impl DockItem {
    fn icon(&self) -> &'static str {
        match self {
            DockItem::Recenter  => "◎",
            DockItem::Files     => "📁",
            DockItem::Firefox   => "🦊",
            DockItem::Tabs      => "▦",
            DockItem::Keyboard  => "⌨",
            DockItem::Stereo3D  => "🥽",
            DockItem::SeekBack  => "⏪",
            DockItem::PlayPause => "⏯",
            DockItem::SeekFwd   => "⏩",
            DockItem::Settings  => "⚙",
            DockItem::Exit      => "✕",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            DockItem::Recenter  => "Recenter",
            DockItem::Files     => "Files",
            DockItem::Firefox   => "Browser",
            DockItem::Tabs      => "Tabs",
            DockItem::Keyboard  => "Keyboard",
            DockItem::Stereo3D  => "3D Mode",
            DockItem::SeekBack  => "-10s",
            DockItem::PlayPause => "Play/Pause",
            DockItem::SeekFwd   => "+10s",
            DockItem::Settings  => "Settings",
            DockItem::Exit      => "Exit VR",
        }
    }
}

// ── File browser / Media Center ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum MediaKind { Dir, Video, Audio }

/// Top-level media category (visionOS-style tabs).
#[derive(Clone, Copy, PartialEq)]
pub enum Category { Movies, Music, Files }

#[derive(Clone)]
pub struct FileEntry {
    pub name:    String,
    pub path:    PathBuf,
    pub is_dir:  bool,
    pub kind:    MediaKind,
    pub size_mb: f32,
    pub thumbnail: Option<egui::TextureHandle>,
    pub glow:      Option<[u8; 3]>, // ambient colour from the poster frame
    pub thumb_requested: bool,
}

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum SortBy { Name, Size, Date }

pub struct FileBrowser {
    pub current_path:   PathBuf,
    pub entries:        Vec<FileEntry>,
    pub selected_index: usize,
    pub error_msg:      Option<String>,
    pub search_query:   String,
    pub sort_by:        SortBy,
    pub category:       Category,
    // Coverflow carousel animation + left-stick repeat.
    pub carousel_pos:   f32,
    pub nav_cooldown:   u8,
    pub nav_hold:       u16,
    /// In-flight recursive media scan (Movies / Music). The walk runs on a worker
    /// thread — `/storage/emulated/0` can be tens of thousands of files and the
    /// render thread must not wait on it.
    scan_rx:            Option<std::sync::mpsc::Receiver<Vec<FileEntry>>>,
}

/// Where the recursive media scan starts, and how far it is allowed to go.
const MEDIA_ROOT: &str = "/storage/emulated/0";
const SCAN_MAX_DEPTH: usize = 5;
const SCAN_MAX_FILES: usize = 600;

/// How far from the cursor a poster is allowed to stay resident.
///
/// A 320x180 RGBA poster costs ~225 KB as a GPU texture. The recursive Movies
/// scan returns hundreds of entries (147 on this device), and posters were
/// never released once decoded: that held ~129 MB against a 256 MB heap limit,
/// and the resulting GC destroyed egui textures while the renderer was still
/// submitting draws referencing them — `DestroyedResource` spam followed by a
/// SIGSEGV in `android_main`.
///
/// The coverflow only draws tiles within 3.4 slots of the cursor (~8 cards), so
/// keeping this many either side is generous while staying bounded.
const THUMB_KEEP_RADIUS: usize = 16;

/// Directories that are never worth walking: app sandboxes, caches and thumbnail
/// stores hold no user media but plenty of files.
fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(name,
            "Android" | "LOST.DIR" | "cache" | "Cache" | "obb" | "data"
            | "Notifications" | "Ringtones" | "Alarms" | "System Volume Information")
}

fn media_kind_of(name: &str) -> Option<MediaKind> {
    let ext = name.rsplit('.').next().map(|e| e.to_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "mp4"|"mkv"|"avi"|"webm"|"mov"|"m4v"|"3gp"|"ts"|"flv") {
        Some(MediaKind::Video)
    } else if matches!(ext.as_str(), "mp3"|"flac"|"wav"|"aac"|"ogg"|"m4a"|"opus"|"wma") {
        Some(MediaKind::Audio)
    } else { None }
}

/// Bounded recursive walk collecting files of one kind.
///
/// The top level of `/storage/emulated/0` holds no media at all — everything is a
/// folder down (Download, Movies, DCIM, …), which is why the Movies/Music tabs
/// used to come up empty. Depth and file count are both capped so a pathological
/// tree cannot turn this into a hang.
fn scan_media(dir: &std::path::Path, want: MediaKind, out: &mut Vec<FileEntry>, depth: usize) {
    if out.len() >= SCAN_MAX_FILES || depth > SCAN_MAX_DEPTH { return; }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        if out.len() >= SCAN_MAX_FILES { return; }
        let path = entry.path();
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if is_dir {
            if !skip_dir(&name) { subdirs.push(path); }
        } else if !name.starts_with('.') && media_kind_of(&name) == Some(want) {
            let size_mb = std::fs::metadata(&path)
                .map(|m| m.len() as f32 / 1_048_576.0).unwrap_or(0.0);
            out.push(FileEntry { name, path, is_dir: false, kind: want,
                size_mb, thumbnail: None, glow: None, thumb_requested: false });
        }
    }
    for sub in subdirs {
        scan_media(&sub, want, out, depth + 1);
        if out.len() >= SCAN_MAX_FILES { return; }
    }
}

impl FileBrowser {
    pub fn new() -> Self {
        let start = PathBuf::from("/storage/emulated/0");
        let mut b = Self {
            current_path:   start,
            entries:        Vec::new(),
            selected_index: 0,
            error_msg:      None,
            search_query:   String::new(),
            sort_by:        SortBy::Name,
            category:       Category::Movies,
            carousel_pos:   0.0,
            nav_cooldown:   0,
            nav_hold:       0,
            scan_rx:        None,
        };
        b.refresh_entries();
        b
    }

    /// Switch category. Movies/Music aggregate media recursively; Files stays a
    /// literal directory browser, so the two need different scans.
    pub fn set_category(&mut self, cat: Category) {
        if self.category == cat { return; }
        self.category = cat;
        self.selected_index = 0;
        self.carousel_pos = 0.0;
        self.refresh_entries();
    }

    /// Pick up a finished background scan. Called once per frame while the Media
    /// Center is on screen.
    pub fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        match rx.try_recv() {
            Ok(mut found) => {
                found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                if self.sort_by == SortBy::Size {
                    found.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb)
                        .unwrap_or(std::cmp::Ordering::Equal));
                }
                log::info!("FileBrowser: recursive scan found {} media file(s)", found.len());
                self.entries = found;
                self.selected_index = 0;
                self.carousel_pos = 0.0;
                self.error_msg = if self.entries.is_empty() {
                    Some("No media found under /storage/emulated/0.".into())
                } else { None };
                self.scan_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.scan_rx = None; }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// True while a background media scan is running (drives the "Scanning…" copy).
    pub fn scanning(&self) -> bool { self.scan_rx.is_some() }

    pub fn refresh_entries(&mut self) {
        use log::{info, error};

        // Movies / Music are aggregations, not directories: kick off a bounded
        // recursive walk on a worker thread and show the result when it lands.
        let want = match self.category {
            Category::Movies => Some(MediaKind::Video),
            Category::Music  => Some(MediaKind::Audio),
            Category::Files  => None,
        };
        if let Some(want) = want {
            self.entries.clear();
            self.selected_index = 0;
            self.error_msg = None;
            let (tx, rx) = std::sync::mpsc::channel();
            self.scan_rx = Some(rx);
            std::thread::spawn(move || {
                let mut out = Vec::new();
                scan_media(std::path::Path::new(MEDIA_ROOT), want, &mut out, 0);
                let _ = tx.send(out);
            });
            info!("FileBrowser: recursive {} scan started under {}",
                if want == MediaKind::Video { "video" } else { "audio" }, MEDIA_ROOT);
            return;
        }
        self.scan_rx = None;

        let prev_path = self.entries.get(self.selected_index).map(|e| e.path.clone());
        self.entries.clear();
        self.selected_index = 0;
        self.error_msg = None;
        info!("FileBrowser: scanning {:?}", self.current_path);

        match std::fs::read_dir(&self.current_path) {
            Ok(rd) => {
                if self.current_path != PathBuf::from("/storage/emulated/0") {
                    if let Some(parent) = self.current_path.parent() {
                        self.entries.push(FileEntry {
                            name: "..".into(), path: parent.to_path_buf(), is_dir: true,
                            kind: MediaKind::Dir, size_mb: 0.0, thumbnail: None,
                            glow: None, thumb_requested: false,
                        });
                    }
                }
                let mut dirs: Vec<FileEntry> = Vec::new();
                let mut files: Vec<FileEntry> = Vec::new();
                for entry in rd.flatten() {
                    let path = entry.path();
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if name.starts_with('.') { continue; }
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    if is_dir {
                        dirs.push(FileEntry { name, path, is_dir: true, kind: MediaKind::Dir,
                            size_mb: 0.0, thumbnail: None, glow: None, thumb_requested: false });
                    } else {
                        let ext = name.rsplit('.').next().map(|e| e.to_lowercase()).unwrap_or_default();
                        let kind = if matches!(ext.as_str(),
                                "mp4"|"mkv"|"avi"|"webm"|"mov"|"m4v"|"3gp"|"ts"|"flv") {
                            Some(MediaKind::Video)
                        } else if matches!(ext.as_str(),
                                "mp3"|"flac"|"wav"|"aac"|"ogg"|"m4a"|"opus"|"wma") {
                            Some(MediaKind::Audio)
                        } else { None };
                        if let Some(kind) = kind {
                            let size_mb = std::fs::metadata(&path).map(|m| m.len() as f32 / 1_048_576.0).unwrap_or(0.0);
                            files.push(FileEntry { name, path, is_dir: false, kind,
                                size_mb, thumbnail: None, glow: None, thumb_requested: false });
                        }
                    }
                }
                dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                match self.sort_by {
                    SortBy::Size => files.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal)),
                    _ => files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                }
                info!("FileBrowser: {} dirs, {} media", dirs.len(), files.len());
                self.entries.extend(dirs);
                self.entries.extend(files);
            }
            Err(e) => {
                error!("FileBrowser: {}", e);
                self.error_msg = Some("Cannot access folder.\nGrant storage permission in Settings.".into());
            }
        }

        if let Some(p) = prev_path {
            if let Some(idx) = self.entries.iter().position(|e| e.path == p) {
                self.selected_index = idx;
            }
        }
        let fi = self.filtered_indices();
        self.carousel_pos = fi.iter().position(|&i| i == self.selected_index).unwrap_or(0) as f32;
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let q = self.search_query.to_lowercase();
        self.entries.iter().enumerate()
            .filter(|(_, e)| {
                let cat_ok = e.is_dir || match self.category {
                    Category::Movies => e.kind == MediaKind::Video,
                    Category::Music  => e.kind == MediaKind::Audio,
                    Category::Files  => true,
                };
                cat_ok && (q.is_empty() || e.name.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Left-stick coverflow sweep with acceleration.
    pub fn handle_stick(&mut self, lx: f32) {
        if lx.abs() < 0.5 {
            self.nav_hold = 0;
            if self.nav_cooldown > 0 { self.nav_cooldown -= 1; }
            return;
        }
        self.nav_hold = self.nav_hold.saturating_add(1);
        if self.nav_cooldown > 0 { self.nav_cooldown -= 1; return; }
        if lx > 0.0 { self.move_down(); } else { self.move_up(); }
        self.nav_cooldown = if self.nav_hold > 28 { 2 } else if self.nav_hold > 10 { 4 } else { 8 };
    }

    /// Inclusive index window around the cursor that may hold posters.
    fn thumb_window(&self) -> (usize, usize) {
        let lo = self.selected_index.saturating_sub(THUMB_KEEP_RADIUS);
        let hi = self.selected_index.saturating_add(THUMB_KEEP_RADIUS)
            .min(self.entries.len().saturating_sub(1));
        (lo, hi)
    }

    /// Release posters that have scrolled far from the cursor.
    ///
    /// Dropping the `TextureHandle` is what actually frees the GPU texture, and
    /// clearing `thumb_requested` lets the tile be re-decoded if the user scrolls
    /// back. Without this the cache grows with the whole scan result and the
    /// process is eventually killed by the allocator, not by egui.
    pub fn evict_distant_thumbnails(&mut self) {
        if self.entries.is_empty() { return; }
        let (lo, hi) = self.thumb_window();
        let mut freed = 0usize;
        for (i, e) in self.entries.iter_mut().enumerate() {
            if (i < lo || i > hi) && e.thumbnail.is_some() {
                e.thumbnail = None;
                e.glow = None;
                e.thumb_requested = false;
                freed += 1;
            }
        }
        if freed > 0 {
            log::info!("FileBrowser: evicted {} off-screen thumbnail(s)", freed);
        }
    }

    /// Video paths still needing a thumbnail (marks them requested).
    ///
    /// Only tiles inside the keep-window are requested: decoding all several
    /// hundred scan results is what exhausted the heap.
    pub fn pending_thumbnail_requests(&mut self, max: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if self.entries.is_empty() { return out; }
        let (lo, hi) = self.thumb_window();
        for e in self.entries[lo..=hi].iter_mut() {
            if e.kind == MediaKind::Video && !e.thumb_requested && e.thumbnail.is_none() {
                e.thumb_requested = true;
                out.push(e.path.clone());
                if out.len() >= max { break; }
            }
        }
        out
    }

    pub fn set_thumbnail(&mut self, path: &std::path::Path, tex: egui::TextureHandle, glow: [u8; 3]) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.path == path) {
            e.thumbnail = Some(tex);
            e.glow = Some(glow);
        }
    }

    pub fn move_up(&mut self) {
        let idx = self.filtered_indices();
        if let Some(pos) = idx.iter().position(|&i| i == self.selected_index) {
            if pos > 0 { self.selected_index = idx[pos - 1]; }
        }
    }
    pub fn move_down(&mut self) {
        let idx = self.filtered_indices();
        if let Some(pos) = idx.iter().position(|&i| i == self.selected_index) {
            if pos + 1 < idx.len() { self.selected_index = idx[pos + 1]; }
        }
    }
    /// Open the highlighted entry. Directories are entered in place; a media file
    /// is RETURNED to the caller, which turns it into an `Intent::PlayFile` — the
    /// browser never holds a "pending selection" flag of its own.
    pub fn select_current(&mut self) -> Option<PathBuf> {
        let entry = self.entries.get(self.selected_index).cloned()?;
        if entry.is_dir {
            self.current_path = entry.path;
            self.search_query.clear();
            self.refresh_entries();
            None
        } else {
            Some(entry.path)
        }
    }
    pub fn go_back(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            if self.current_path != PathBuf::from("/storage/emulated/0") {
                self.current_path = parent.to_path_buf();
                self.search_query.clear();
                self.refresh_entries();
            }
        }
    }
}

// ── Web browser state ─────────────────────────────────────────────────────────

#[allow(dead_code)]
pub const VIEWPORTS: [(i32, i32, &str); 4] = [
    (1920, 1080, "Wide"),
    (1080, 1920, "Tall"),
    (1080, 2160, "Phone"),
    (1200, 1200, "Square"),
];

/// Angular size of the browser plane relative to the (square) UI panel.
///
/// The page is drawn as its own curved screen — height `1.6 * zoom` at radius 5.3 —
/// while the egui panel is `1.7 * zoom` at radius 2.0. Both curve identically, so the
/// ratio of their angular spans is constant and the zoom cancels:
///   (1.6 / 5.3) / (1.7 / 2.0) = 0.3552
/// Multiply by the 2048-px panel to get the page's footprint inside the UI texture —
/// that is what makes the cursor land where the user sees it.
pub const PAGE_TO_PANEL: f32 = 0.355166;
pub const UI_PANEL_PX: f32 = 2048.0;

/// The page rectangle inside the 2048² UI canvas, for a page of aspect `aspect`.
pub fn page_rect(aspect: f32) -> egui::Rect {
    let h = PAGE_TO_PANEL * UI_PANEL_PX;
    let w = h * aspect.max(0.1);
    egui::Rect::from_center_size(
        egui::pos2(UI_PANEL_PX * 0.5, UI_PANEL_PX * 0.5),
        egui::vec2(w, h))
}

/// Aspect ratio of a "16:9"-style label from the Java side.
pub fn aspect_from_label(label: &str) -> f32 {
    let mut it = label.split(':');
    let a: f32 = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(16.0);
    let b: f32 = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(9.0);
    if b <= 0.0 { 16.0 / 9.0 } else { a / b }
}

pub struct WebBrowserState {
    pub url_bar:        String,
    pub current_url:    String,
    #[allow(dead_code)]
    pub mic_listening:  bool,
    #[allow(dead_code)]
    pub viewport:       u8,
    pub launched:       bool,

    // ── Tab model (mirrored from Java a few times a second) ──────────────────
    pub tabs:           Vec<crate::webview::TabInfo>,
    pub active_tab:     usize,
    pub progress:       i32,
    pub text_focused:   bool,
    /// Aspect ratio of the ACTIVE tab's viewport, drives cursor/page mapping.
    pub plane_aspect:   f32,
    pub info_tick:      u32,

    // ── Stick cursor ─────────────────────────────────────────────────────────
    /// Cursor position in normalized page coordinates (0..1, y down).
    pub cursor_x:       f32,
    pub cursor_y:       f32,
    pub click_flash:    u8,

    // ── Tab overview (Safari-style grid) ─────────────────────────────────────
    /// Highlighted card in the grid. Visibility of the grid itself is `Focus`.
    pub overview_sel:     usize,
    /// Java's tab cap, mirrored so the UI can explain a refused new tab.
    pub max_tabs:         usize,
    /// Transient engine message ("Tab limit reached…"), mirrored from Java.
    pub notice:           String,
    /// Per-tab previews, keyed by tab index: (sequence, uploaded texture). The
    /// sequence is Java's; when it changes we re-upload, otherwise we reuse the
    /// texture, so the overview costs one small upload per second at most.
    pub thumbs:           std::collections::HashMap<usize, (u32, egui::TextureHandle)>,
}

impl Default for WebBrowserState {
    fn default() -> Self {
        Self {
            url_bar: "https://www.google.com".into(),
            current_url: String::new(),
            mic_listening: false,
            viewport: 0, launched: false,
            tabs: Vec::new(),
            active_tab: 0,
            progress: 100,
            text_focused: false,
            plane_aspect: 16.0 / 9.0,
            info_tick: 0,
            cursor_x: 0.5, cursor_y: 0.5,
            click_flash: 0,
            overview_sel: 0,
            max_tabs: 0,
            notice: String::new(),
            thumbs: std::collections::HashMap::new(),
        }
    }
}

impl WebBrowserState {
    /// Right stick drives the cursor. Small deflections creep (precision), full
    /// deflection sweeps the page in well under a second.
    pub fn move_cursor(&mut self, sx: f32, sy: f32) {
        const DEAD: f32 = 0.15;
        let mag = (sx * sx + sy * sy).sqrt();
        if mag < DEAD { return; }
        let norm = ((mag - DEAD) / (1.0 - DEAD)).clamp(0.0, 1.0);
        // Quadratic ramp + a small linear floor: fine control near the deadzone,
        // fast travel at the rim.
        let speed = 0.0022 * norm + 0.020 * norm * norm;
        let (ux, uy) = (sx / mag, sy / mag);
        // The page is wider than tall, so equal angular speed needs the x step
        // divided by the aspect to feel isotropic.
        self.cursor_x = (self.cursor_x + ux * speed / self.plane_aspect.max(0.2)).clamp(0.0, 1.0);
        self.cursor_y = (self.cursor_y + uy * speed).clamp(0.0, 1.0);
    }

    /// Adopt a fresh snapshot of the Java-side tab model.
    pub fn apply_snapshot(&mut self, snap: crate::webview::TabSnapshot) {
        self.active_tab   = snap.active;
        self.progress     = snap.progress;
        self.text_focused = snap.text_focused;
        self.tabs         = snap.tabs;
        self.max_tabs     = snap.max_tabs;
        self.notice       = snap.notice;
        // Previews are keyed by tab index; drop any that no longer exist so a
        // closed tab's picture can't reappear under a different tab.
        let n = self.tabs.len();
        self.thumbs.retain(|k, _| *k < n);
        if let Some(t) = self.tabs.get(self.active_tab) {
            self.plane_aspect = aspect_from_label(&t.aspect);
            if !t.url.is_empty() { self.current_url = t.url.clone(); }
        }
        if self.overview_sel >= self.tabs.len() {
            self.overview_sel = self.tabs.len().saturating_sub(1);
        }
    }

    pub fn overview_move(&mut self, delta: i32) {
        if self.tabs.is_empty() { return; }
        let n = self.tabs.len() as i32;
        self.overview_sel = (((self.overview_sel as i32 + delta) % n + n) % n) as usize;
    }
}

// ── In-VR virtual keyboard (gamepad-driven) ───────────────────────────────────

const KB_ROWS: [&str; 4] = [
    "1234567890",
    "qwertyuiop",
    "asdfghjkl",
    "zxcvbnm",
];

/// The bottom action row, mirroring an Android IME: a wide space bar, a delete
/// key, and the accented Search/Go key that commits the buffer. It is a normal
/// row as far as D-pad navigation is concerned (`row == KB_ROWS.len()`), so it is
/// reachable by pressing down from "zxcvbnm", and each key is also clickable.
const KB_ACTION_ROW: usize = KB_ROWS.len();
const KB_ACTIONS: [(&str, KeyAction); 4] = [
    ("space", KeyAction::Space),
    (".",     KeyAction::Dot),
    ("⌫",     KeyAction::Backspace),
    ("Search ⏎", KeyAction::Search),
];

#[derive(Clone, Copy, PartialEq)]
pub enum KeyAction { Space, Dot, Backspace, Search }

/// What pressing a key asked the surrounding UI to do. `Commit` is the only one
/// the caller has to act on (it routes to browser navigation).
#[derive(Clone, Copy, PartialEq)]
pub enum KeyPress { Handled, Commit }

#[derive(Default)]
pub struct VrKeyboard {
    pub row: usize,
    pub col: usize,
    pub input: String,
}

impl VrKeyboard {
    fn current_char(&self) -> Option<char> {
        KB_ROWS.get(self.row).and_then(|r| r.chars().nth(self.col))
    }
    /// Number of keys in a row, action row included.
    fn row_len(row: usize) -> usize {
        if row == KB_ACTION_ROW { KB_ACTIONS.len() }
        else { KB_ROWS.get(row).map(|r| r.chars().count()).unwrap_or(1) }
    }
    pub fn move_left(&mut self)  { if self.col > 0 { self.col -= 1; } }
    pub fn move_right(&mut self) {
        if self.col + 1 < Self::row_len(self.row) { self.col += 1; }
    }
    pub fn move_up(&mut self)   { if self.row > 0 { self.row -= 1; self.clamp_col(); } }
    pub fn move_down(&mut self) {
        if self.row < KB_ACTION_ROW { self.row += 1; self.clamp_col(); }
    }
    fn clamp_col(&mut self) {
        let last = Self::row_len(self.row).saturating_sub(1);
        if self.col > last { self.col = last; }
    }
    /// Type the highlighted key. Returns `Commit` for the Search/Go key so the
    /// caller can route the buffer to the browser.
    pub fn press(&mut self) -> KeyPress {
        if self.row == KB_ACTION_ROW {
            return match KB_ACTIONS.get(self.col).map(|(_, a)| *a) {
                Some(KeyAction::Space)     => { self.input.push(' '); KeyPress::Handled }
                Some(KeyAction::Dot)       => { self.input.push('.'); KeyPress::Handled }
                Some(KeyAction::Backspace) => { self.input.pop(); KeyPress::Handled }
                Some(KeyAction::Search)    => KeyPress::Commit,
                None => KeyPress::Handled,
            };
        }
        if let Some(c) = self.current_char() { self.input.push(c); }
        KeyPress::Handled
    }
    pub fn backspace(&mut self) { self.input.pop(); }
    /// Hand the typed string to the caller and reset. The keyboard owns no
    /// "committed" flag of its own — the caller turns this into an intent.
    pub fn submit(&mut self) -> String { std::mem::take(&mut self.input) }

    /// Draw the keyboard. Returns `Some(Commit)` when a click (cursor) landed on
    /// the Search key, so the caller can commit exactly as the gamepad path does.
    fn render(&mut self, ui: &mut egui::Ui) -> Option<KeyPress> {
        let mut out = None;
        for (r, row) in KB_ROWS.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (c, ch) in row.chars().enumerate() {
                    let selected = r == self.row && c == self.col;
                    let size = if selected { 72.0 } else { 60.0 };
                    let label = egui::RichText::new(ch.to_string())
                        .size(if selected { 36.0 } else { 26.0 })
                        .color(if selected { M3_ON_PRIMARY } else { M3_ON_SURFACE });
                    let btn = egui::Button::new(label)
                        .min_size(egui::vec2(size, size))
                        // Same shape morph as the dock: rounded square -> pill.
                        .rounding(Rounding::same(if selected { size / 2.0 } else { 18.0 }))
                        .fill(if selected { M3_PRIMARY } else { M3_SURFACE_HIGH });
                    let resp = ui.add(btn);
                    if resp.hovered() { self.row = r; self.col = c; }
                    if resp.clicked() { self.row = r; self.col = c; out = Some(self.press()); }
                }
            });
        }
        // Action row: space / . / delete / the accented Search key.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (c, (label, act)) in KB_ACTIONS.iter().enumerate() {
                let selected = self.row == KB_ACTION_ROW && self.col == c;
                let is_go = *act == KeyAction::Search;
                let w = match act {
                    KeyAction::Space  => 250.0,
                    KeyAction::Search => 230.0,
                    _                 => 90.0,
                };
                let h = if selected { 72.0 } else { 60.0 };
                // The Go key is the primary action, so it carries the accent
                // even when it is not the selected key — exactly how Android's
                // IME enter key reads.
                let (bg, fg) = if selected { (M3_PRIMARY, M3_ON_PRIMARY) }
                    else if is_go { (M3_PRIMARY, M3_ON_PRIMARY) }
                    else { (M3_SURFACE_HIGH, M3_ON_SURFACE) };
                let btn = egui::Button::new(
                        egui::RichText::new(*label).size(if is_go { 28.0 } else { 24.0 })
                            .strong().color(fg))
                    .min_size(egui::vec2(w, h))
                    .rounding(Rounding::same(if selected { h / 2.0 } else { 18.0 }))
                    .stroke(if selected {
                        Stroke::new(3.0, Color32::from_white_alpha(150))
                    } else { Stroke::NONE })
                    .fill(bg);
                let resp = ui.add(btn);
                if resp.hovered() { self.row = KB_ACTION_ROW; self.col = c; }
                if resp.clicked() {
                    self.row = KB_ACTION_ROW; self.col = c;
                    out = Some(self.press());
                }
            }
        });
        out
    }
}

// ── VrUi ──────────────────────────────────────────────────────────────────────

pub struct VrUi {
    pub params: VrParams,
    /// THE authority on who owns input. Panel visibility is derived from it.
    focus: Focus,
    pub menu_state: MenuState,
    pub hamburger_visible: bool,
    last_interaction: Instant,
    pub file_browser: FileBrowser,
    pub web_browser: WebBrowserState,
    pub keyboard: VrKeyboard,
    pub dock_selected: usize,
    /// One-shot commands awaiting exactly one drain by lib.rs. A plain queue is
    /// enough — the UI is single-threaded and lives entirely on the render thread,
    /// so no lock or atomic is warranted here.
    intents: std::collections::VecDeque<Intent>,

    // ── Panel pointer ─────────────────────────────────────────────────────────
    //
    // The headset has no touchscreen, so egui never received a pointer and every
    // on-screen control (tab close ✕, ＋ New, keyboard keys) was dead — only the
    // gamepad bindings did anything. The right stick now drives a pointer in
    // panel-space and we synthesise egui pointer events for it.
    /// Pointer position inside the 2048² UI canvas.
    ui_cursor: egui::Pos2,
    /// Synthesised egui events awaiting exactly one drain by lib.rs — the same
    /// queue-and-drain discipline as `intents`, so nothing can fire twice.
    pointer_events: Vec<egui::Event>,
    /// Set once per frame from `ctx.wants_pointer_input()`: true when the pointer
    /// is over an interactive widget. Purely a cache of egui's own answer, read by
    /// lib.rs so the ✕ button drives the widget instead of the gamepad binding.
    pointer_hot: bool,
}

impl VrUi {
    pub fn new(ctx: &Context) -> Self {
        // Pin the UI scale so layout is independent of the device's (high) DPI —
        // the UI renders into a fixed square texture; ppp=1.0 uses the full space.
        ctx.set_pixels_per_point(1.0);
        Self::apply_theme(ctx);
        Self {
            params: VrParams::default(),
            focus: Focus::Video,
            menu_state: MenuState::Main,
            hamburger_visible: true,
            last_interaction: Instant::now(),
            file_browser: FileBrowser::new(),
            web_browser: WebBrowserState::default(),
            keyboard: VrKeyboard::default(),
            dock_selected: 0,
            intents: std::collections::VecDeque::new(),
            ui_cursor: egui::pos2(UI_PANEL_PX * 0.5, UI_PANEL_PX * 0.5),
            pointer_events: Vec::new(),
            pointer_hot: false,
        }
    }

    // ── Panel pointer ─────────────────────────────────────────────────────────

    /// True while a panel (not the page / bare video) owns input, i.e. while the
    /// synthesised pointer should exist at all.
    pub fn pointer_active(&self) -> bool {
        matches!(self.focus,
            Focus::Dock | Focus::MediaCenter | Focus::Keyboard | Focus::TabOverview)
    }

    /// Is the pointer over a clickable widget right now? (egui's own verdict.)
    pub fn pointer_hot(&self) -> bool { self.pointer_hot }

    /// Right stick moves the panel pointer. Same feel as the browser cursor:
    /// quadratic ramp for precision near the deadzone, fast travel at the rim.
    pub fn move_ui_cursor(&mut self, sx: f32, sy: f32) {
        const DEAD: f32 = 0.15;
        let mag = (sx * sx + sy * sy).sqrt();
        if mag < DEAD { return; }
        let norm = ((mag - DEAD) / (1.0 - DEAD)).clamp(0.0, 1.0);
        let speed = (6.0 * norm + 46.0 * norm * norm) * UI_PANEL_PX / 2048.0;
        self.ui_cursor.x = (self.ui_cursor.x + sx / mag * speed).clamp(0.0, UI_PANEL_PX);
        self.ui_cursor.y = (self.ui_cursor.y + sy / mag * speed).clamp(0.0, UI_PANEL_PX);
    }

    /// Synthesise a full click (down+up) at the pointer.
    pub fn ui_click(&mut self) {
        let pos = self.ui_cursor;
        for pressed in [true, false] {
            self.pointer_events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    /// Events for this frame's `RawInput`. Drained exactly once by lib.rs.
    pub fn take_pointer_events(&mut self) -> Vec<egui::Event> {
        let mut ev = std::mem::take(&mut self.pointer_events);
        if self.pointer_active() {
            // Keep hover alive: egui forgets the pointer without a move event.
            ev.insert(0, egui::Event::PointerMoved(self.ui_cursor));
        } else {
            ev.push(egui::Event::PointerGone);
        }
        ev
    }

    fn apply_theme(ctx: &Context) {
        let mut style = Style::default();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
        style.spacing.slider_width = 160.0;
        let mut visuals = Visuals::dark();
        visuals.window_rounding = Rounding::same(18.0);
        style.text_styles.insert(egui::TextStyle::Body, FontId::new(16.0, FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Button, FontId::new(20.0, FontFamily::Proportional));
        ctx.set_style(style);
        ctx.set_visuals(visuals);
    }

    pub fn show_hamburger(&mut self) { self.hamburger_visible = true; self.last_interaction = Instant::now(); }
    #[allow(dead_code)]
    pub fn toggle_hamburger(&mut self) { self.hamburger_visible = !self.hamburger_visible; self.last_interaction = Instant::now(); }
    #[allow(dead_code)]
    pub fn is_hamburger_visible(&self) -> bool { self.hamburger_visible }

    // ── Focus / intents ───────────────────────────────────────────────────────

    pub fn focus(&self) -> Focus { self.focus }

    /// Where focus returns to when a panel is dismissed: the browser page if it is
    /// on screen, otherwise the video surface.
    pub fn base_focus(&self) -> Focus {
        if self.params.web_mode { Focus::Browser } else { Focus::Video }
    }

    pub fn set_focus(&mut self, f: Focus) {
        self.focus = f;
        self.last_interaction = Instant::now();
    }

    /// Every opener is a toggle: pressing the same button again returns to base.
    pub fn toggle_focus(&mut self, f: Focus) {
        let next = if self.focus == f { self.base_focus() } else { f };
        self.set_focus(next);
    }

    /// Derived visibility — never stored, so it cannot disagree with `focus`.
    pub fn media_visible(&self) -> bool { self.focus == Focus::MediaCenter }

    /// Queue a one-shot command for lib.rs.
    pub fn push(&mut self, i: Intent) { self.intents.push_back(i); }

    /// Take every queued command. Called exactly once per frame by lib.rs.
    pub fn drain_intents(&mut self) -> Vec<Intent> { self.intents.drain(..).collect() }

    /// R2 / ✕ — click at the cursor. Java's `tap` takes NORMALIZED (0..1) page
    /// coordinates, which is exactly what the cursor stores.
    pub fn browser_click(&mut self) {
        let (x, y) = (self.web_browser.cursor_x, self.web_browser.cursor_y);
        self.web_browser.click_flash = 8;
        self.push(Intent::Tap(x, y));
        // Clicking an input field pops the VR keyboard on the next sync, once Gecko
        // reports the focus change (see `sync_tabs`).
    }

    /// Left stick — scroll the page under the cursor.
    pub fn browser_scroll(&mut self, sx: f32, sy: f32) {
        const DEAD: f32 = 0.2;
        if sx.abs() < DEAD && sy.abs() < DEAD { return; }
        let f = |v: f32| if v.abs() < DEAD { 0.0 } else { v * v.abs() * 34.0 };
        let (cx, cy) = (self.web_browser.cursor_x, self.web_browser.cursor_y);
        self.push(Intent::Scroll(-f(sx), -f(sy), cx, cy));
    }

    /// Adopt a fresh snapshot of the Java-owned tab model (see WebBrowserState).
    /// This is the ONLY place the Rust mirror of tab state is written.
    pub fn sync_tabs(&mut self, snap: crate::webview::TabSnapshot) {
        let was_focused = self.web_browser.text_focused;
        self.web_browser.apply_snapshot(snap);
        // Focused-text-field detection: the page just took text focus while the
        // browser has the controller → open the VR keyboard for it.
        if !was_focused && self.web_browser.text_focused && self.focus == Focus::Browser {
            self.set_focus(Focus::Keyboard);
        }
    }

    /// Sequence number of the cached preview for tab `i`, if we have one. lib.rs
    /// compares it against Java's to decide whether a re-upload is needed.
    pub fn tab_thumb_seq(&self, i: usize) -> Option<u32> {
        self.web_browser.thumbs.get(&i).map(|(s, _)| *s)
    }

    pub fn set_tab_thumb(&mut self, i: usize, seq: u32, tex: egui::TextureHandle) {
        self.web_browser.thumbs.insert(i, (seq, tex));
    }

    /// Open the highlighted Media Center entry (directory or file).
    pub fn media_select(&mut self) {
        if let Some(path) = self.file_browser.select_current() {
            self.set_focus(self.base_focus());
            self.push(Intent::PlayFile(path));
        }
    }

    /// Commit the keyboard buffer: navigate the browser, or type into a focused
    /// web text field when the page asked for input.
    pub fn keyboard_commit(&mut self) {
        let text = self.keyboard.submit();
        self.set_focus(self.base_focus());
        if text.trim().is_empty() { return; }
        if self.params.web_mode && self.web_browser.text_focused {
            self.push(Intent::TypeText(text));
            self.push(Intent::SubmitEnter);
            return;
        }
        let target = normalise_url(&text);
        self.web_browser.url_bar = target.clone();
        if !self.params.web_mode {
            self.open_browser();
        }
        self.push(Intent::Navigate(target));
    }

    // ── Dock navigation (D-pad driven; wired from lib.rs) ─────────────────────
    pub fn dock_move_left(&mut self)  { if self.dock_selected > 0 { self.dock_selected -= 1; } }
    pub fn dock_move_right(&mut self) { if self.dock_selected + 1 < DOCK_ITEMS.len() { self.dock_selected += 1; } }

    pub fn dock_activate(&mut self) {
        match DOCK_ITEMS[self.dock_selected] {
            DockItem::Recenter  => self.push(Intent::Recenter),
            DockItem::Files     => {
                if self.file_browser.entries.is_empty() { self.file_browser.refresh_entries(); }
                self.set_focus(Focus::MediaCenter);
            }
            DockItem::Firefox   => self.toggle_browser(),
            DockItem::Tabs      => {
                if self.params.web_mode {
                    self.web_browser.overview_sel = self.web_browser.active_tab;
                    self.set_focus(Focus::TabOverview);
                } else {
                    self.toggle_browser();
                }
            }
            DockItem::Keyboard  => self.set_focus(Focus::Keyboard),
            DockItem::Stereo3D  => {
                self.params.stereo_mode = (self.params.stereo_mode + 1) % STEREO_MODES;
            }
            DockItem::SeekBack  => self.push(Intent::Seek(-10_000_000)),
            DockItem::PlayPause => self.push(Intent::TogglePlayPause),
            DockItem::SeekFwd   => self.push(Intent::Seek(10_000_000)),
            DockItem::Settings  => self.menu_state = MenuState::LensSettings,
            DockItem::Exit      => self.push(Intent::ExitVr),
        }
    }

    /// Browser button: show the page and hand it focus, or put it away again.
    pub fn toggle_browser(&mut self) {
        if self.params.web_mode {
            self.params.web_mode = false;
            self.menu_state = MenuState::Main;
            self.set_focus(Focus::Video);
            return;
        }
        self.open_browser();
    }

    pub fn open_browser(&mut self) {
        self.params.web_mode = true;
        self.menu_state = MenuState::WebBrowser;
        self.push(Intent::SetEngine(self.params.browser_engine));
        if !self.web_browser.launched {
            self.web_browser.launched = true;
            let url = if self.web_browser.current_url.is_empty() {
                self.web_browser.url_bar.clone()
            } else { self.web_browser.current_url.clone() };
            self.push(Intent::Navigate(url));
        }
        self.set_focus(Focus::Browser);
    }

    // ── Render ────────────────────────────────────────────────────────────────
    pub fn render(&mut self, ctx: &Context, vr_mode_active: bool) {
        if !vr_mode_active { return; }
        ctx.set_pixels_per_point(1.0);

        if self.params.web_mode {
            self.render_web_chrome(ctx);
        }
        match self.focus {
            Focus::Dock        => self.render_main_dock(ctx),
            Focus::MediaCenter => self.render_media_center(ctx),
            Focus::Keyboard    => self.render_keyboard(ctx),
            Focus::TabOverview => self.render_tab_overview(ctx),
            Focus::Video | Focus::Browser => {}
        }

        // Panel pointer: draw it on top of whatever panel is open, and remember
        // egui's verdict on whether it is over a widget (see `pointer_hot`).
        if self.pointer_active() {
            let p = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground, egui::Id::new("ui_cursor")));
            let c = self.ui_cursor;
            p.circle_filled(c, 14.0, Color32::from_black_alpha(120));
            p.circle_filled(c, 9.0, if self.pointer_hot { M3_ON_PRIMARY } else { M3_PRIMARY });
            p.circle_stroke(c, 14.0, Stroke::new(2.5, Color32::from_white_alpha(210)));
        }
        self.pointer_hot = ctx.wants_pointer_input();
    }

    // ── macOS-style dock ──────────────────────────────────────────────────────
    fn render_main_dock(&mut self, ctx: &Context) {
        if let MenuState::LensSettings = self.menu_state {
            self.render_lens_settings(ctx);
            return;
        }
        // Material 3 Expressive: a tonal "surface container" bar with a fully-rounded
        // (pill) outer shape, and shape morphing on the selection - the unselected
        // items are squircle-ish rounded squares, the selected one swells and morphs
        // to a full pill in the M3E accent colour. Emphasis comes from size + shape +
        // tone together, which is the core of the expressive spec.
        egui::Window::new("dock")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::symmetric(24.0, 20.0))
                .rounding(Rounding::same(64.0))
                .stroke(Stroke::new(1.5, Color32::from_rgba_unmultiplied(208, 188, 255, 45)))
                .fill(M3_SURFACE_CONTAINER))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        for (i, item) in DOCK_ITEMS.iter().enumerate() {
                            let selected = i == self.dock_selected;
                            let toggled = matches!(item, DockItem::Firefox if self.params.web_mode)
                                || matches!(item, DockItem::Keyboard if self.focus == Focus::Keyboard)
                                || matches!(item, DockItem::Tabs if self.focus == Focus::TabOverview);
                            let is_exit = *item == DockItem::Exit;

                            let size = if selected { 132.0 } else { 100.0 };
                            let icon_size = if selected { 68.0 } else { 48.0 };
                            // Shape morph: rounded-square at rest -> pill when selected.
                            let radius = if selected { size / 2.0 } else { 32.0 };

                            let (bg, icon_col) = if selected {
                                if is_exit { (M3_ERROR, M3_ON_ERROR) }
                                else { (M3_PRIMARY, M3_ON_PRIMARY) }
                            } else if toggled {
                                (M3_SECONDARY_CONTAINER, M3_ON_SECONDARY_CONTAINER)
                            } else if is_exit {
                                (M3_SURFACE_HIGH, M3_ERROR_SOFT)
                            } else {
                                (M3_SURFACE_HIGH, M3_ON_SURFACE)
                            };

                            let btn = egui::Button::new(
                                    egui::RichText::new(item.icon()).size(icon_size).color(icon_col))
                                .min_size(egui::vec2(size, size))
                                .rounding(Rounding::same(radius))
                                .stroke(if selected {
                                    Stroke::new(2.0, Color32::from_white_alpha(60))
                                } else { Stroke::NONE })
                                .fill(bg);
                            let resp = ui.add(btn);
                            if resp.clicked() { self.dock_selected = i; self.dock_activate(); }
                            if resp.hovered() { self.dock_selected = i; }
                        }
                    });

                    ui.add_space(14.0);
                    let sel = DOCK_ITEMS[self.dock_selected];
                    let label = if sel == DockItem::Stereo3D {
                        stereo_label(self.params.stereo_mode)
                    } else { sel.label() };
                    // The head-tracking basis mode is surfaced here so the user can
                    // cycle it with D-pad down and report which one tracks correctly.
                    let sub = format!("{}   ·   ↑ geometry   ↓ look [{}]",
                        projection_label(self.params.projection_mode),
                        crate::sensors::head_mode_label());
                    // M3E display-style label: large, tight, high-contrast.
                    ui.label(egui::RichText::new(label).size(34.0).strong().color(M3_ON_SURFACE));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(sub).size(19.0)
                        .color(Color32::from_rgb(160, 154, 168)));
                });
            });
    }

    fn render_lens_settings(&mut self, ctx: &Context) {
        egui::Window::new("lens_settings")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(20.0))
                .rounding(Rounding::same(24.0))
                .fill(Color32::from_rgba_unmultiplied(24, 24, 32, 240)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("⬅").size(24.0))
                        .min_size(egui::vec2(50.0, 50.0))).clicked() {
                        self.menu_state = MenuState::Main;
                    }
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label("Lens Size");
                        ui.add(egui::Slider::new(&mut self.params.lens_radius, 0.5..=1.5).fixed_decimals(2));
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label("Lens Dist");
                        ui.add(egui::Slider::new(&mut self.params.lens_center_offset, -0.15..=0.15).fixed_decimals(3));
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label("Zoom");
                        ui.add(egui::Slider::new(&mut self.params.content_scale, 0.5..=3.0).fixed_decimals(2));
                    });
                });
            });
    }

    // ── Media Center — Nokia coverflow carousel (light frosted glass) ─────────
    fn render_media_center(&mut self, ctx: &Context) {
        // Adopt a finished background scan before laying anything out.
        self.file_browser.poll_scan();
        if self.file_browser.scanning() { ctx.request_repaint(); }
        let txt    = Color32::from_rgb(26, 26, 32);
        let txt2   = Color32::from_rgb(108, 110, 120);
        let accent = Color32::from_rgb(46, 107, 230);

        egui::Window::new("media_center")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .fixed_size(egui::vec2(980.0, 660.0))
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(26.0))
                .rounding(Rounding::same(30.0))
                .stroke(Stroke::new(1.0, Color32::from_black_alpha(28)))
                .fill(Color32::from_rgba_unmultiplied(238, 240, 244, 216)))
            .show(ctx, |ui| {
                // Title + close
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Media Center").size(26.0).strong().color(txt));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("✕").size(18.0).color(txt))
                            .min_size(egui::vec2(34.0, 34.0)).rounding(Rounding::same(17.0))
                            .fill(Color32::from_black_alpha(16))).clicked() {
                            let back = self.base_focus();
                            self.set_focus(back);
                        }
                    });
                });
                ui.add_space(12.0);
                // Category pills
                ui.horizontal(|ui| {
                    for (cat, label, icon) in [
                        (Category::Movies, "Movies", "🎬"),
                        (Category::Music,  "Music",  "🎵"),
                        (Category::Files,  "Files",  "🗂"),
                    ] {
                        let on = self.file_browser.category == cat;
                        let pill = egui::Button::new(
                                egui::RichText::new(format!("{}  {}", icon, label)).size(15.0)
                                    .color(if on { Color32::WHITE } else { txt2 }))
                            .min_size(egui::vec2(134.0, 40.0)).rounding(Rounding::same(20.0))
                            .fill(if on { accent } else { Color32::from_black_alpha(12) });
                        if ui.add(pill).clicked() { self.file_browser.set_category(cat); }
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(10.0);
                // Breadcrumb: for Movies/Music this is an aggregate, not a folder.
                let path_str = match self.file_browser.category {
                    Category::Files => self.file_browser.current_path.to_string_lossy().to_string(),
                    _ if self.file_browser.scanning() =>
                        format!("Scanning {} …", MEDIA_ROOT),
                    _ => format!("All media under {}", MEDIA_ROOT),
                };
                ui.label(egui::RichText::new(path_str).size(13.0).color(txt2));
                ui.add_space(8.0);

                let indices = self.file_browser.filtered_indices();
                let mut select_index: Option<usize> = None;
                let mut open_index: Option<usize> = None;

                if let Some(err) = self.file_browser.error_msg.clone() {
                    ui.add_space(50.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("⚠  Permission Denied").size(20.0).color(Color32::from_rgb(200, 90, 40)));
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(err).size(13.0).color(txt2));
                    });
                } else if indices.is_empty() {
                    ui.add_space(70.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Nothing here").size(18.0).color(txt2));
                    });
                } else {
                    if !indices.contains(&self.file_browser.selected_index) {
                        self.file_browser.selected_index = indices[0];
                    }
                    let sel_pos = indices.iter().position(|&i| i == self.file_browser.selected_index).unwrap_or(0) as f32;
                    let cp = self.file_browser.carousel_pos;
                    let np = cp + (sel_pos - cp) * 0.22;
                    self.file_browser.carousel_pos = if (np - sel_pos).abs() < 0.002 { sel_pos } else { np };
                    if (self.file_browser.carousel_pos - sel_pos).abs() > 0.002 { ctx.request_repaint(); }
                    let pos = self.file_browser.carousel_pos;

                    let (canvas, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 400.0), egui::Sense::hover());
                    let center = canvas.center() - egui::vec2(0.0, 28.0);
                    let focus_w = 380.0_f32;
                    let focus_h = focus_w * 9.0 / 16.0;

                    let mut order: Vec<(f32, usize, f32)> = indices.iter().enumerate()
                        .map(|(slot, &ei)| { let off = slot as f32 - pos; (off.abs(), ei, off) })
                        .filter(|(a, _, _)| *a <= 3.4)
                        .collect();
                    order.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

                    for (a, ei, off) in order {
                        let entry = &self.file_browser.entries[ei];
                        let focused = a < 0.5;
                        let scale = (1.0 - 0.24 * a).max(0.34);
                        let w = focus_w * scale; let h = w * 9.0 / 16.0;
                        let x = center.x + off * 150.0;
                        let rect = egui::Rect::from_center_size(egui::pos2(x, center.y), egui::vec2(w, h));
                        let alpha = (1.0 - 0.30 * a).clamp(0.35, 1.0);
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

                        if let Some(g) = entry.glow {
                            let ga = (alpha * if focused { 130.0 } else { 60.0 }) as u8;
                            ui.painter().rect_filled(rect.expand(if focused { 11.0 } else { 5.0 }),
                                Rounding::same(18.0), Color32::from_rgba_unmultiplied(g[0], g[1], g[2], ga));
                        }
                        let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
                        if let Some(tex) = &entry.thumbnail {
                            ui.painter().image(tex.id(), rect, uv, tint);
                            if focused {
                                let refl = egui::Rect::from_min_size(
                                    egui::pos2(rect.min.x, rect.max.y + 4.0), egui::vec2(w, h * 0.42));
                                ui.painter().image(tex.id(), refl,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.58)),
                                    Color32::from_white_alpha(38));
                            }
                        } else {
                            ui.painter().rect_filled(rect, Rounding::same(10.0),
                                Color32::from_rgba_unmultiplied(70, 74, 84, (alpha * 220.0) as u8));
                            let glyph = match entry.kind {
                                MediaKind::Dir => "📁", MediaKind::Video => "🎬", MediaKind::Audio => "🎵",
                            };
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph,
                                FontId::new(44.0 * scale, FontFamily::Proportional),
                                Color32::from_white_alpha((alpha * 210.0) as u8));
                        }
                        ui.painter().rect_stroke(rect, Rounding::same(if focused { 6.0 } else { 4.0 }),
                            Stroke::new(if focused { 2.0 } else { 1.0 }, Color32::from_black_alpha((alpha * 55.0) as u8)));

                        if focused && entry.kind == MediaKind::Video && entry.thumbnail.is_some() {
                            let c = rect.center();
                            ui.painter().circle_filled(c, 22.0, Color32::from_black_alpha(120));
                            ui.painter().text(c + egui::vec2(2.0, 0.0), egui::Align2::CENTER_CENTER,
                                "▶", FontId::new(20.0, FontFamily::Proportional), Color32::WHITE);
                        }

                        let resp = ui.interact(rect, ui.id().with(("cover", ei)), egui::Sense::click());
                        // Hovering brings a tile to the front of the coverflow, so
                        // the pointer and the D-pad share one selection.
                        if resp.hovered() && !focused { select_index = Some(ei); }
                        if resp.clicked() {
                            if focused { open_index = Some(ei); } else { select_index = Some(ei); }
                        }
                    }

                    let sel = &self.file_browser.entries[self.file_browser.selected_index];
                    ui.painter().text(egui::pos2(center.x, center.y + focus_h * 0.5 + 38.0),
                        egui::Align2::CENTER_CENTER, &sel.name,
                        FontId::new(19.0, FontFamily::Proportional), txt);
                    let meta = if sel.is_dir { "Folder".to_string() }
                        else if sel.size_mb > 1000.0 { format!("{:.1} GB", sel.size_mb / 1024.0) }
                        else { format!("{:.0} MB", sel.size_mb) };
                    ui.painter().text(egui::pos2(center.x, center.y + focus_h * 0.5 + 62.0),
                        egui::Align2::CENTER_CENTER, &meta,
                        FontId::new(13.0, FontFamily::Proportional), txt2);

                    let n = indices.len();
                    if n > 1 && n <= 40 {
                        let spacing = 14.0; let total = (n as f32 - 1.0) * spacing;
                        let dy = canvas.max.y - 6.0;
                        for k in 0..n {
                            let dx = center.x - total * 0.5 + k as f32 * spacing;
                            let on = (k as f32 - sel_pos).abs() < 0.5;
                            ui.painter().circle_filled(egui::pos2(dx, dy), if on { 3.6 } else { 2.2 },
                                if on { accent } else { Color32::from_black_alpha(55) });
                        }
                    }
                }

                if let Some(ei) = select_index { self.file_browser.selected_index = ei; }
                if open_index.is_some() { self.media_select(); }

                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("◀ ▶ / left-stick: browse    X: open    ○: up a folder    △: close")
                        .size(12.0).color(txt2));
                });
            });
    }

    // ── Browser chrome (hugs the page plane) ─────────────────────────────────
    //
    // Everything here is positioned against `page_rect`, the footprint the browser
    // plane actually occupies inside the square UI canvas, so the chrome sits just
    // above/below the page instead of floating at the far edge of the UI panel.
    fn render_web_chrome(&mut self, ctx: &Context) {
        let page = page_rect(self.web_browser.plane_aspect);

        // ── Tab strip + URL + progress, above the page ───────────────────────
        let n_tabs = self.web_browser.tabs.len();
        let active = self.web_browser.active_tab;
        let url = if self.web_browser.current_url.is_empty() {
            self.web_browser.url_bar.clone()
        } else { self.web_browser.current_url.clone() };
        let progress = self.web_browser.progress;
        let notice   = self.web_browser.notice.clone();
        let aspect_label = self.web_browser.tabs.get(active)
            .map(|t| t.aspect.clone()).unwrap_or_else(|| "16:9".into());
        let titles: Vec<String> = self.web_browser.tabs.iter().enumerate()
            .map(|(i, t)| {
                let name = if t.title.trim().is_empty() { short_host(&t.url) } else { t.title.clone() };
                format!("{}  {}", i + 1, truncate(&name, 18))
            }).collect();

        let mut want_select: Option<usize> = None;
        egui::Window::new("web_chrome")
            .fixed_pos(egui::pos2(page.min.x, page.min.y - 132.0))
            .fixed_size(egui::vec2(page.width(), 108.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::symmetric(16.0, 10.0))
                .rounding(Rounding::same(26.0))
                .stroke(Stroke::new(1.5, Color32::from_rgba_unmultiplied(208, 188, 255, 45)))
                .fill(M3_SURFACE_CONTAINER))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (i, t) in titles.iter().enumerate() {
                        let on = i == active;
                        let pill = egui::Button::new(egui::RichText::new(t).size(17.0)
                                .color(if on { M3_ON_PRIMARY } else { M3_ON_SURFACE }))
                            .min_size(egui::vec2(0.0, 40.0))
                            .rounding(Rounding::same(20.0))
                            .fill(if on { M3_PRIMARY } else { M3_SURFACE_HIGH });
                        if ui.add(pill).clicked() { want_select = Some(i); }
                    }
                    if ui.add(egui::Button::new(egui::RichText::new("＋").size(20.0).color(M3_ON_SECONDARY_CONTAINER))
                        .min_size(egui::vec2(46.0, 40.0)).rounding(Rounding::same(20.0))
                        .fill(M3_SECONDARY_CONTAINER)).clicked() {
                        self.push(Intent::NewTab);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("▭ {}", aspect_label))
                            .size(17.0).color(M3_PRIMARY));
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🔒").size(15.0).color(Color32::from_rgb(148, 143, 153)));
                    ui.label(egui::RichText::new(truncate(&url, 90)).size(18.0).color(M3_ON_SURFACE));
                });
                // Engine feedback, e.g. a new tab refused at the cap — otherwise
                // the ＋ button just looks broken.
                if !notice.trim().is_empty() {
                    ui.label(egui::RichText::new(&notice).size(17.0).strong().color(M3_PRIMARY));
                }
                // Loading progress
                if progress < 100 {
                    let (r, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 5.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, Rounding::same(3.0), M3_SURFACE_HIGH);
                    let done = egui::Rect::from_min_size(r.min,
                        egui::vec2(r.width() * (progress as f32 / 100.0), r.height()));
                    ui.painter().rect_filled(done, Rounding::same(3.0), M3_PRIMARY);
                    ctx.request_repaint();
                }
            });
        if let Some(i) = want_select {
            self.push(Intent::SelectTab(i));
        }

        // ── Cursor + page outline ────────────────────────────────────────────
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground, egui::Id::new("web_cursor")));
        painter.rect_stroke(page.expand(3.0), Rounding::same(8.0),
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(208, 188, 255, 60)));
        let cx = page.min.x + self.web_browser.cursor_x * page.width();
        let cy = page.min.y + self.web_browser.cursor_y * page.height();
        let c = egui::pos2(cx, cy);
        let flash = self.web_browser.click_flash;
        if flash > 0 {
            self.web_browser.click_flash -= 1;
            painter.circle_filled(c, 22.0, Color32::from_rgba_unmultiplied(208, 188, 255, 90));
            ctx.request_repaint();
        }
        // Halo + dot: readable over any page content, light and dark alike.
        painter.circle_filled(c, 13.0, Color32::from_black_alpha(120));
        painter.circle_filled(c, 9.0, M3_PRIMARY);
        painter.circle_stroke(c, 13.0, Stroke::new(2.0, Color32::from_white_alpha(200)));

        // ── Control hints, below the page ────────────────────────────────────
        let hint = format!(
            "R-stick cursor   R2/✕ click   L-stick scroll   L1/R1 tab   □ tabs   ▷ shape [{}]   ○ back   △ keyboard   Options dock",
            aspect_label);
        egui::Window::new("web_hints")
            .fixed_pos(egui::pos2(page.min.x, page.max.y + 22.0))
            .fixed_size(egui::vec2(page.width(), 44.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::symmetric(16.0, 8.0))
                .rounding(Rounding::same(22.0))
                .fill(Color32::from_rgba_unmultiplied(24, 24, 32, 225)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if Self::icon_btn(ui, "←").clicked() { self.push(Intent::BrowserBack); }
                    if Self::icon_btn(ui, "→").clicked() { self.push(Intent::BrowserForward); }
                    if Self::icon_btn(ui, "↺").clicked() { self.push(Intent::BrowserReload); }
                    if Self::icon_btn(ui, "▦").clicked() {
                        self.web_browser.overview_sel = active.min(n_tabs.saturating_sub(1));
                        self.set_focus(Focus::TabOverview);
                    }
                    if Self::icon_btn(ui, "🎬").clicked() { self.toggle_browser(); }
                    ui.label(egui::RichText::new(hint).size(16.0)
                        .color(Color32::from_rgb(160, 154, 168)));
                });
            });
    }

    // ── Global tab overview (Safari-style grid) ──────────────────────────────
    fn render_tab_overview(&mut self, ctx: &Context) {
        let sel = self.web_browser.overview_sel;
        let active = self.web_browser.active_tab;
        // Snapshot everything the grid needs (previews included) up front, so the
        // closure below can still call `self.push(...)` without a borrow fight.
        let tabs: Vec<(String, String, String, Option<egui::TextureHandle>)> =
            self.web_browser.tabs.iter().enumerate()
                .map(|(i, t)| (t.title.clone(), t.url.clone(), t.aspect.clone(),
                               self.web_browser.thumbs.get(&i).map(|(_, tex)| tex.clone())))
                .collect();
        let notice = self.web_browser.notice.clone();
        let max_tabs = self.web_browser.max_tabs;
        let mut pick: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut hover: Option<usize> = None;

        egui::Window::new("tab_overview")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .fixed_size(egui::vec2(1220.0, 760.0))
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(30.0))
                .rounding(Rounding::same(38.0))
                .stroke(Stroke::new(1.5, Color32::from_rgba_unmultiplied(208, 188, 255, 45)))
                .fill(M3_SURFACE_CONTAINER))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Tabs").size(38.0).strong().color(M3_ON_SURFACE));
                    ui.add_space(14.0);
                    let count = if max_tabs > 0 {
                        format!("{} of {} open", tabs.len(), max_tabs)
                    } else { format!("{} open", tabs.len()) };
                    ui.label(egui::RichText::new(count)
                        .size(20.0).color(Color32::from_rgb(160, 154, 168)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let full = max_tabs > 0 && tabs.len() >= max_tabs;
                        if ui.add(egui::Button::new(egui::RichText::new("＋ New").size(20.0)
                                .color(if full { Color32::from_rgb(140, 136, 148) }
                                       else { M3_ON_PRIMARY }))
                            .min_size(egui::vec2(140.0, 52.0)).rounding(Rounding::same(26.0))
                            .fill(if full { M3_SURFACE_HIGH } else { M3_PRIMARY })).clicked() {
                            // Even at the cap: Java answers with a notice, which is
                            // what turns a dead button into an explanation.
                            self.push(Intent::NewTab);
                            if !full { self.set_focus(Focus::Browser); }
                        }
                    });
                });
                // Engine feedback (tab cap reached, last tab reset, …).
                if !notice.trim().is_empty() {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(M3_SECONDARY_CONTAINER)
                        .rounding(Rounding::same(16.0))
                        .inner_margin(Margin::symmetric(16.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&notice).size(19.0)
                                .color(M3_ON_SECONDARY_CONTAINER));
                        });
                }
                ui.add_space(20.0);

                // Card grid. Each card's preview box takes the shape of THAT tab's
                // own aspect ratio, so differently-shaped tabs read differently.
                const COLS: usize = 2;
                const CARD_W: f32 = 550.0;
                const BOX_H: f32 = 250.0;
                for row in 0..((tabs.len() + COLS - 1) / COLS).max(1) {
                    ui.horizontal(|ui| {
                        for col in 0..COLS {
                            let i = row * COLS + col;
                            let Some((title, url, aspect, thumb)) = tabs.get(i) else { continue };
                            let is_sel = i == sel;
                            let (card, _) = ui.allocate_exact_size(
                                egui::vec2(CARD_W, BOX_H + 96.0), egui::Sense::hover());
                            let p = ui.painter();
                            p.rect_filled(card, Rounding::same(24.0),
                                if is_sel { M3_SECONDARY_CONTAINER } else { M3_SURFACE_HIGH });
                            if is_sel {
                                p.rect_stroke(card, Rounding::same(24.0), Stroke::new(3.0, M3_PRIMARY));
                            }
                            let a = aspect_from_label(aspect);
                            let bh = BOX_H - 24.0;
                            let bw = (bh * a).min(CARD_W - 48.0);
                            let bh = bw / a;
                            let bx = egui::Rect::from_center_size(
                                egui::pos2(card.center().x, card.min.y + 16.0 + BOX_H * 0.5),
                                egui::vec2(bw, bh));
                            p.rect_filled(bx, Rounding::same(14.0), Color32::from_rgb(38, 36, 44));
                            let host = short_host(url);
                            // Real preview: the last frame this tab painted while it
                            // was in front (Java keeps a downscaled copy, since a
                            // backgrounded tab has no compositor surface at all).
                            if let Some(tex) = thumb {
                                p.image(tex.id(), bx,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    Color32::WHITE);
                            } else {
                                p.text(bx.center(), egui::Align2::CENTER_CENTER,
                                    host.chars().next().unwrap_or('•').to_uppercase().to_string(),
                                    FontId::new(64.0, FontFamily::Proportional),
                                    Color32::from_rgb(120, 116, 130));
                            }
                            p.rect_stroke(bx, Rounding::same(14.0),
                                Stroke::new(1.5, Color32::from_white_alpha(30)));
                            p.text(egui::pos2(bx.max.x - 8.0, bx.min.y + 8.0),
                                egui::Align2::RIGHT_TOP, aspect,
                                FontId::new(17.0, FontFamily::Proportional), M3_PRIMARY);
                            if i == active {
                                p.text(egui::pos2(bx.min.x + 10.0, bx.min.y + 8.0),
                                    egui::Align2::LEFT_TOP, "● live",
                                    FontId::new(16.0, FontFamily::Proportional), M3_PRIMARY);
                            }
                            let name = if title.trim().is_empty() { host.clone() } else { title.clone() };
                            p.text(egui::pos2(card.min.x + 24.0, card.min.y + BOX_H + 34.0),
                                egui::Align2::LEFT_CENTER, truncate(&name, 34),
                                FontId::new(22.0, FontFamily::Proportional), M3_ON_SURFACE);
                            p.text(egui::pos2(card.min.x + 24.0, card.min.y + BOX_H + 64.0),
                                egui::Align2::LEFT_CENTER, truncate(&host, 40),
                                FontId::new(17.0, FontFamily::Proportional),
                                Color32::from_rgb(160, 154, 168));

                            // Close affordance
                            let x_c = egui::pos2(card.max.x - 30.0, card.min.y + 30.0);
                            p.circle_filled(x_c, 20.0, Color32::from_black_alpha(140));
                            p.text(x_c, egui::Align2::CENTER_CENTER, "✕",
                                FontId::new(20.0, FontFamily::Proportional), M3_ERROR);
                            // Close first: its hit box sits inside the card, so it
                            // has to win the interaction, and the card must only be
                            // consulted when the ✕ was not hit.
                            let x_resp = ui.interact(
                                egui::Rect::from_center_size(x_c, egui::vec2(56.0, 56.0)),
                                ui.id().with(("tabx", i)), egui::Sense::click());
                            let card_resp =
                                ui.interact(card, ui.id().with(("tab", i)), egui::Sense::click());
                            if x_resp.hovered() {
                                p.circle_stroke(x_c, 22.0, Stroke::new(2.5, M3_ERROR));
                            }
                            // Hovering a card moves the D-pad highlight with it, so
                            // the pointer and the gamepad never disagree.
                            if x_resp.hovered() || card_resp.hovered() { hover = Some(i); }
                            if x_resp.clicked() {
                                close = Some(i);
                            } else if card_resp.clicked() {
                                pick = Some(i);
                            }
                            ui.add_space(16.0);
                        }
                    });
                    ui.add_space(16.0);
                }

                ui.add_space(8.0);
                ui.label(egui::RichText::new(
                        "R-stick pointer   ✕ open / click   ○ close tab   □ new tab   △ back to page")
                    .size(18.0).color(Color32::from_rgb(160, 154, 168)));
            });

        if let Some(i) = hover { self.web_browser.overview_sel = i; }
        if let Some(i) = pick {
            self.push(Intent::SelectTab(i));
            self.set_focus(Focus::Browser);
        }
        if let Some(i) = close {
            self.web_browser.overview_sel =
                i.min(self.web_browser.tabs.len().saturating_sub(2));
            self.push(Intent::CloseTabAt(i));
        }
    }

    fn render_keyboard(&mut self, ctx: &Context) {
        let mut hit: Option<KeyPress> = None;
        egui::Window::new("keyboard")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(22.0))
                .rounding(Rounding::same(36.0))
                .stroke(Stroke::new(1.5, Color32::from_rgba_unmultiplied(208, 188, 255, 45)))
                .fill(M3_SURFACE_CONTAINER))
            .show(ctx, |ui| {
                // M3 text field: filled tonal surface, always visible so it's obvious
                // where the typed text is going even before anything is typed.
                egui::Frame::none()
                    .fill(M3_SURFACE_HIGH)
                    .rounding(Rounding::same(16.0))
                    .inner_margin(Margin::symmetric(18.0, 12.0))
                    .show(ui, |ui| {
                        ui.set_min_width(600.0);
                        let (text, col) = if self.keyboard.input.is_empty() {
                            ("Search or enter address".to_string(), Color32::from_rgb(148, 143, 153))
                        } else {
                            (format!("{}|", self.keyboard.input), M3_ON_SURFACE)
                        };
                        ui.label(egui::RichText::new(text).size(26.0).color(col));
                    });
                ui.add_space(14.0);
                hit = self.keyboard.render(ui);
                ui.add_space(10.0);
                ui.label(egui::RichText::new(
                        "X type   ○ delete   ↓ to Search key   Options go   △ close")
                    .size(18.0).color(Color32::from_rgb(148, 143, 153)));
            });
        if hit == Some(KeyPress::Commit) { self.keyboard_commit(); }
    }

    fn icon_btn(ui: &mut egui::Ui, icon: &str) -> egui::Response {
        ui.add(egui::Button::new(egui::RichText::new(icon).size(22.0))
            .min_size(egui::vec2(48.0, 44.0))
            .fill(Color32::from_rgba_unmultiplied(40, 40, 55, 200)))
    }
}

/// Shorten for display, with an ellipsis when clipped.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { return s.to_string(); }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Host part of a URL, without scheme or "www.".
pub fn short_host(url: &str) -> String {
    let s = url.split("://").nth(1).unwrap_or(url);
    let host = s.split('/').next().unwrap_or(s);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

pub fn normalise_url(input: &str) -> String {
    let s = input.trim();
    if s.starts_with("http://") || s.starts_with("https://") { return s.to_string(); }
    if !s.contains(' ') && (s.contains('.') || s.starts_with("localhost")) {
        return format!("https://{}", s);
    }
    format!("https://www.google.com/search?q={}", s.replace(' ', "+"))
}
