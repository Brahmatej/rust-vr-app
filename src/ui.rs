use egui::{Context, Visuals, Style, Rounding, Color32, Margin, Stroke, FontId, FontFamily};
use std::time::Instant;
use std::path::PathBuf;

// ── VR tunable parameters ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct VrParams {
    pub lens_radius:        f32,
    pub lens_center_offset: f32,
    pub content_scale:      f32,
    pub target_scale:       f32,   // lerp target for smooth zoom
    pub gyro_enabled:       bool,
    pub recenter_flag:      bool,
    pub select_video_flag:  bool,
    pub vr_exit_requested:  bool,
    // Playback
    pub toggle_play_pause:  bool,
    pub seek_forward_flag:  bool,
    pub seek_backward_flag: bool,
    // Web mode
    pub web_mode:           bool,
    pub browser_engine:     i32,        // 0 = Chromium (unused), 1 = Firefox/Gecko
    pub pending_engine:     Option<i32>,
    // Stereoscopic video layout: 0 = mono, 1 = SBS, 2 = over-under.
    pub stereo_mode:        u8,
}

impl Default for VrParams {
    fn default() -> Self {
        Self {
            lens_radius:        1.0,
            lens_center_offset: 0.0,
            content_scale:      1.0,
            target_scale:       1.0,
            gyro_enabled:       true,
            recenter_flag:      false,
            select_video_flag:  false,
            vr_exit_requested:  false,
            toggle_play_pause:  false,
            seek_forward_flag:  false,
            seek_backward_flag: false,
            web_mode:           false,
            browser_engine:     1,
            pending_engine:     None,
            stereo_mode:        0,
        }
    }
}

pub const STEREO_MODES: u8 = 3;

pub fn stereo_label(mode: u8) -> &'static str {
    match mode { 1 => "3D · Side-by-Side", 2 => "3D · Over-Under", _ => "2D · Mono" }
}

pub enum MenuState { Main, LensSettings, WebBrowser }

// ── macOS-style center dock ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum DockItem {
    Recenter,
    Gyro,
    Files,
    Web,
    Firefox,
    NewTab,
    CloseTab,
    Stereo3D,
    SeekBack,
    PlayPause,
    SeekFwd,
    Settings,
    Exit,
}

pub const DOCK_ITEMS: [DockItem; 13] = [
    DockItem::Recenter,
    DockItem::Gyro,
    DockItem::Files,
    DockItem::Web,
    DockItem::Firefox,
    DockItem::NewTab,
    DockItem::CloseTab,
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
            DockItem::Gyro      => "🧭",
            DockItem::Files     => "📁",
            DockItem::Web       => "🌐",
            DockItem::Firefox   => "🦊",
            DockItem::NewTab    => "➕",
            DockItem::CloseTab  => "⊝",
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
            DockItem::Gyro      => "Gyro",
            DockItem::Files     => "Files",
            DockItem::Web       => "Web",
            DockItem::Firefox   => "Firefox",
            DockItem::NewTab    => "New Tab",
            DockItem::CloseTab  => "Close Tab",
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
pub enum SortBy { Name, Size, Date }

pub struct FileBrowser {
    pub visible:        bool,
    pub current_path:   PathBuf,
    pub entries:        Vec<FileEntry>,
    pub selected_index: usize,
    pub selected_file:  Option<PathBuf>,
    pub error_msg:      Option<String>,
    pub search_query:   String,
    pub sort_by:        SortBy,
    pub category:       Category,
    // Coverflow carousel animation + left-stick repeat.
    pub carousel_pos:   f32,
    pub nav_cooldown:   u8,
    pub nav_hold:       u16,
}

impl FileBrowser {
    pub fn new() -> Self {
        let start = PathBuf::from("/storage/emulated/0");
        let mut b = Self {
            visible:        false,
            current_path:   start,
            entries:        Vec::new(),
            selected_index: 0,
            selected_file:  None,
            error_msg:      None,
            search_query:   String::new(),
            sort_by:        SortBy::Name,
            category:       Category::Movies,
            carousel_pos:   0.0,
            nav_cooldown:   0,
            nav_hold:       0,
        };
        b.refresh_entries();
        b
    }

    pub fn refresh_entries(&mut self) {
        use log::{info, error};
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

    /// Video paths still needing a thumbnail (marks them requested).
    pub fn pending_thumbnail_requests(&mut self, max: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in self.entries.iter_mut() {
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
    pub fn select_current(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index).cloned() {
            if entry.is_dir {
                self.current_path = entry.path;
                self.search_query.clear();
                self.refresh_entries();
            } else {
                self.selected_file = Some(entry.path);
                self.visible = false;
            }
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
    pub fn take_selected_file(&mut self) -> Option<PathBuf> {
        self.selected_file.take()
    }
}

// ── Web browser state ─────────────────────────────────────────────────────────

pub const VIEWPORTS: [(i32, i32, &str); 4] = [
    (1920, 1080, "Wide"),
    (1080, 1920, "Tall"),
    (1080, 2160, "Phone"),
    (1200, 1200, "Square"),
];

pub struct WebBrowserState {
    pub url_bar:        String,
    pub current_url:    String,
    pub pending_url:    Option<String>,
    pub mic_listening:  bool,
    pub go_back:        bool,
    pub go_forward:     bool,
    pub reload:         bool,
    pub new_tab:        bool,
    pub close_tab:      bool,
    pub viewport:       u8,
    pub pending_resize: Option<(i32, i32)>,
    pub launched:       bool,
}

impl Default for WebBrowserState {
    fn default() -> Self {
        Self {
            url_bar: "https://www.google.com".into(),
            current_url: String::new(),
            pending_url: None,
            mic_listening: false,
            go_back: false, go_forward: false, reload: false,
            new_tab: false, close_tab: false,
            viewport: 0, pending_resize: None, launched: false,
        }
    }
}

// ── In-VR virtual keyboard (gamepad-driven) ───────────────────────────────────

const KB_ROWS: [&str; 4] = [
    "1234567890",
    "qwertyuiop",
    "asdfghjkl",
    "zxcvbnm",
];

#[derive(Default)]
pub struct VrKeyboard {
    pub visible: bool,
    pub row: usize,
    pub col: usize,
    pub input: String,
    pub commit: Option<String>,
}

impl VrKeyboard {
    fn current_char(&self) -> Option<char> {
        KB_ROWS.get(self.row).and_then(|r| r.chars().nth(self.col))
    }
    pub fn move_left(&mut self)  { if self.col > 0 { self.col -= 1; } }
    pub fn move_right(&mut self) {
        let len = KB_ROWS[self.row].chars().count();
        if self.col + 1 < len { self.col += 1; }
    }
    pub fn move_up(&mut self)   { if self.row > 0 { self.row -= 1; self.clamp_col(); } }
    pub fn move_down(&mut self) { if self.row + 1 < KB_ROWS.len() { self.row += 1; self.clamp_col(); } }
    fn clamp_col(&mut self) {
        let len = KB_ROWS[self.row].chars().count().saturating_sub(1);
        if self.col > len { self.col = len; }
    }
    pub fn press(&mut self) {
        if let Some(c) = self.current_char() { self.input.push(c); }
    }
    pub fn backspace(&mut self) { self.input.pop(); }
    pub fn submit(&mut self) {
        self.commit = Some(std::mem::take(&mut self.input));
        self.visible = false;
    }
    pub fn take_commit(&mut self) -> Option<String> { self.commit.take() }

    fn render(&self, ui: &mut egui::Ui) {
        for (r, row) in KB_ROWS.iter().enumerate() {
            ui.horizontal(|ui| {
                for (c, ch) in row.chars().enumerate() {
                    let selected = r == self.row && c == self.col;
                    let label = egui::RichText::new(ch.to_string())
                        .size(if selected { 34.0 } else { 26.0 })
                        .color(Color32::WHITE);
                    let mut btn = egui::Button::new(label).min_size(egui::vec2(64.0, 64.0));
                    if selected { btn = btn.fill(Color32::from_rgb(80, 160, 255)); }
                    ui.add(btn);
                }
            });
        }
    }
}

// ── VrUi ──────────────────────────────────────────────────────────────────────

pub struct VrUi {
    pub params: VrParams,
    pub main_menu_visible: bool,
    pub menu_state: MenuState,
    pub hamburger_visible: bool,
    last_interaction: Instant,
    pub file_browser: FileBrowser,
    pub web_browser: WebBrowserState,
    pub keyboard: VrKeyboard,
    pub dock_selected: usize,
}

impl VrUi {
    pub fn new(ctx: &Context) -> Self {
        // Pin the UI scale so layout is independent of the device's (high) DPI —
        // the UI renders into a fixed square texture; ppp=1.0 uses the full space.
        ctx.set_pixels_per_point(1.0);
        Self::apply_theme(ctx);
        Self {
            params: VrParams::default(),
            main_menu_visible: false,
            menu_state: MenuState::Main,
            hamburger_visible: true,
            last_interaction: Instant::now(),
            file_browser: FileBrowser::new(),
            web_browser: WebBrowserState::default(),
            keyboard: VrKeyboard::default(),
            dock_selected: 0,
        }
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
    pub fn toggle_hamburger(&mut self) { self.hamburger_visible = !self.hamburger_visible; self.last_interaction = Instant::now(); }
    pub fn is_hamburger_visible(&self) -> bool { self.hamburger_visible }

    pub fn take_selected_file(&mut self) -> Option<PathBuf> { self.file_browser.take_selected_file() }

    // ── Dock navigation (D-pad driven; wired from lib.rs) ─────────────────────
    pub fn dock_move_left(&mut self)  { if self.dock_selected > 0 { self.dock_selected -= 1; } }
    pub fn dock_move_right(&mut self) { if self.dock_selected + 1 < DOCK_ITEMS.len() { self.dock_selected += 1; } }

    pub fn dock_activate(&mut self) {
        match DOCK_ITEMS[self.dock_selected] {
            DockItem::Recenter  => self.params.recenter_flag = true,
            DockItem::Gyro      => self.params.gyro_enabled = !self.params.gyro_enabled,
            DockItem::Files     => {
                self.file_browser.visible = true;
                if self.file_browser.entries.is_empty() { self.file_browser.refresh_entries(); }
                self.main_menu_visible = false;
            }
            DockItem::Web | DockItem::Firefox => self.activate_browser(1),
            DockItem::NewTab    => { if !self.params.web_mode { self.activate_browser(1); } self.web_browser.new_tab = true; self.main_menu_visible = false; }
            DockItem::CloseTab  => self.web_browser.close_tab = true,
            DockItem::Stereo3D  => {
                self.params.stereo_mode = (self.params.stereo_mode + 1) % STEREO_MODES;
            }
            DockItem::SeekBack  => self.params.seek_backward_flag = true,
            DockItem::PlayPause => self.params.toggle_play_pause = true,
            DockItem::SeekFwd   => self.params.seek_forward_flag = true,
            DockItem::Settings  => self.menu_state = MenuState::LensSettings,
            DockItem::Exit      => self.params.vr_exit_requested = true,
        }
    }

    fn activate_browser(&mut self, engine: i32) {
        if self.params.web_mode { self.params.web_mode = false; return; }
        self.params.web_mode = true;
        self.params.browser_engine = engine;
        self.params.pending_engine = Some(engine);
        self.menu_state = MenuState::WebBrowser;
        if !self.web_browser.launched {
            self.web_browser.launched = true;
            let url = if self.web_browser.current_url.is_empty() {
                self.web_browser.url_bar.clone()
            } else { self.web_browser.current_url.clone() };
            self.web_browser.pending_url = Some(url);
        }
    }

    // ── Render ────────────────────────────────────────────────────────────────
    pub fn render(&mut self, ctx: &Context, vr_mode_active: bool) {
        if !vr_mode_active { return; }
        ctx.set_pixels_per_point(1.0);

        if self.main_menu_visible {
            self.render_main_dock(ctx);
        }
        if self.file_browser.visible {
            self.render_media_center(ctx);
        }
        if self.params.web_mode {
            self.render_web_toolbar(ctx);
        }
        if self.keyboard.visible {
            self.render_keyboard(ctx);
        }
    }

    // ── macOS-style dock ──────────────────────────────────────────────────────
    fn render_main_dock(&mut self, ctx: &Context) {
        if let MenuState::LensSettings = self.menu_state {
            self.render_lens_settings(ctx);
            return;
        }
        egui::Window::new("dock")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(18.0))
                .rounding(Rounding::same(28.0))
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(30)))
                .fill(Color32::from_rgba_unmultiplied(24, 24, 32, 235)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        for (i, item) in DOCK_ITEMS.iter().enumerate() {
                            let selected = i == self.dock_selected;
                            let toggled = matches!(item, DockItem::Gyro if self.params.gyro_enabled)
                                || matches!(item, DockItem::Web | DockItem::Firefox if self.params.web_mode);
                            let size = if selected { 100.0 } else { 74.0 };
                            let icon_size = if selected { 52.0 } else { 36.0 };
                            let bg = if selected { Color32::from_rgb(70, 140, 250) }
                                else if toggled { Color32::from_rgb(45, 90, 150) }
                                else { Color32::from_rgba_unmultiplied(45, 45, 58, 230) };
                            let icon_col = if *item == DockItem::Exit && !selected {
                                Color32::from_rgb(255, 110, 110)
                            } else { Color32::WHITE };
                            let btn = egui::Button::new(
                                    egui::RichText::new(item.icon()).size(icon_size).color(icon_col))
                                .min_size(egui::vec2(size, size))
                                .rounding(Rounding::same(20.0))
                                .fill(bg);
                            let resp = ui.add(btn);
                            if resp.clicked() { self.dock_selected = i; self.dock_activate(); }
                            if resp.hovered() { self.dock_selected = i; }
                        }
                    });
                    ui.add_space(10.0);
                    let sel = DOCK_ITEMS[self.dock_selected];
                    let label = if sel == DockItem::Stereo3D {
                        stereo_label(self.params.stereo_mode)
                    } else { sel.label() };
                    ui.label(egui::RichText::new(label).size(26.0).strong().color(Color32::WHITE));
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
                            self.file_browser.visible = false;
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
                        if ui.add(pill).clicked() {
                            self.file_browser.category = cat;
                            self.file_browser.selected_index = 0;
                        }
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(10.0);
                // Breadcrumb
                let path_str = self.file_browser.current_path.to_string_lossy().to_string();
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
                if open_index.is_some() { self.file_browser.select_current(); }

                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("◀ ▶ / left-stick: browse    X: open    ○: up a folder    △: close")
                        .size(12.0).color(txt2));
                });
            });
    }

    // ── Web toolbar ───────────────────────────────────────────────────────────
    fn render_web_toolbar(&mut self, ctx: &Context) {
        egui::Window::new("web_toolbar")
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(10.0))
                .rounding(Rounding::same(22.0))
                .fill(Color32::from_rgba_unmultiplied(24, 24, 32, 235)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if Self::icon_btn(ui, "←").clicked() { self.web_browser.go_back = true; }
                    if Self::icon_btn(ui, "→").clicked() { self.web_browser.go_forward = true; }
                    if Self::icon_btn(ui, "↺").clicked() { self.web_browser.reload = true; }
                    ui.add(egui::TextEdit::singleline(&mut self.web_browser.url_bar)
                        .desired_width(360.0).hint_text("Enter URL…"));
                    // 3D toggle for VR web content
                    let on = self.params.stereo_mode != 0;
                    let label = match self.params.stereo_mode { 1 => "3D SBS", 2 => "3D OU", _ => "2D" };
                    if ui.add(egui::Button::new(egui::RichText::new(label).size(16.0)
                            .color(if on { Color32::WHITE } else { Color32::from_gray(200) }))
                        .min_size(egui::vec2(72.0, 44.0))
                        .fill(if on { Color32::from_rgb(60, 120, 220) }
                              else { Color32::from_rgba_unmultiplied(40, 40, 55, 200) })).clicked() {
                        self.params.stereo_mode = (self.params.stereo_mode + 1) % STEREO_MODES;
                    }
                    if Self::icon_btn(ui, "🎬").clicked() {
                        self.params.web_mode = false;
                        self.main_menu_visible = false;
                    }
                });
            });
    }

    fn render_keyboard(&mut self, ctx: &Context) {
        egui::Window::new("keyboard")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false).collapsible(false).title_bar(false)
            .frame(egui::Frame::window(&ctx.style())
                .inner_margin(Margin::same(14.0))
                .rounding(Rounding::same(18.0))
                .fill(Color32::from_rgb(18, 18, 24)))
            .show(ctx, |ui| {
                if !self.keyboard.input.is_empty() {
                    ui.label(egui::RichText::new(&self.keyboard.input).size(22.0).color(Color32::WHITE));
                    ui.separator();
                }
                self.keyboard.render(ui);
            });
    }

    fn icon_btn(ui: &mut egui::Ui, icon: &str) -> egui::Response {
        ui.add(egui::Button::new(egui::RichText::new(icon).size(22.0))
            .min_size(egui::vec2(48.0, 44.0))
            .fill(Color32::from_rgba_unmultiplied(40, 40, 55, 200)))
    }
}

pub fn normalise_url(input: &str) -> String {
    let s = input.trim();
    if s.starts_with("http://") || s.starts_with("https://") { return s.to_string(); }
    if !s.contains(' ') && (s.contains('.') || s.starts_with("localhost")) {
        return format!("https://{}", s);
    }
    format!("https://www.google.com/search?q={}", s.replace(' ', "+"))
}
