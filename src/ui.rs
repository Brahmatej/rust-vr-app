use egui::{Context, Visuals, Style, Rounding, Color32, Margin, Stroke, Shadow, FontId, FontFamily};
use std::time::Instant;
use std::path::PathBuf;

/// Tunable parameters for VR renderer and sensors
#[derive(Debug, Clone, Copy)]
pub struct VrParams {
    pub lens_radius: f32,
    pub lens_center_offset: f32,
    pub content_scale: f32,
    pub gyro_enabled: bool,
    pub recenter_flag: bool,
    pub select_video_flag: bool,
    pub vr_exit_requested: bool,
    // Playback controls
    pub toggle_play_pause: bool,
    pub seek_forward_flag: bool,
    pub seek_backward_flag: bool,
}

impl Default for VrParams {
    fn default() -> Self {
        Self {
            lens_radius: 1.0,
            lens_center_offset: 0.0,
            content_scale: 1.0,
            gyro_enabled: true,
            recenter_flag: false,
            select_video_flag: false,
            vr_exit_requested: false,
            toggle_play_pause: false,
            seek_forward_flag: false,
            seek_backward_flag: false,
        }
    }
}

pub enum MenuState {
    Main,
    LensSettings,
}

/// File/folder entry for the browser
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// In-VR File Browser
pub struct FileBrowser {
    pub visible: bool,
    pub current_path: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected_index: usize,
    pub selected_file: Option<PathBuf>,  // Set when user selects a video
    pub error_msg: Option<String>,        // Error message for display
}

impl FileBrowser {
    pub fn new() -> Self {
        let start_path = PathBuf::from("/storage/emulated/0");
        let mut browser = Self {
            visible: false,
            current_path: start_path.clone(),
            entries: Vec::new(),
            selected_index: 0,
            selected_file: None,
            error_msg: None,
        };
        browser.refresh_entries();
        browser
    }
    
    /// Refresh the file list for current directory
    pub fn refresh_entries(&mut self) {
        use log::{info, error};
        
        self.entries.clear();
        self.selected_index = 0;
        self.error_msg = None;
        
        info!("FileBrowser: Refreshing path: {:?}", self.current_path);
        
        // Try to read the directory
        match std::fs::read_dir(&self.current_path) {
            Ok(read_dir) => {
                info!("FileBrowser: Successfully opened directory");
                
                // Add parent directory entry if not at root
                if self.current_path.parent().is_some() && self.current_path != PathBuf::from("/storage/emulated/0") {
                    self.entries.push(FileEntry {
                        name: "📁 ..".to_string(),
                        path: self.current_path.parent().unwrap().to_path_buf(),
                        is_dir: true,
                    });
                }
                
                let mut dirs: Vec<FileEntry> = Vec::new();
                let mut files: Vec<FileEntry> = Vec::new();
                
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    
                    // Skip hidden files
                    if name.starts_with('.') {
                        continue;
                    }
                    
                    // Use file_type from DirEntry (more reliable on Android)
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    
                    info!("FileBrowser: Entry '{}' is_dir={}", name, is_dir);
                    
                    if is_dir {
                        dirs.push(FileEntry {
                            name: format!("📁 {}", name),
                            path,
                            is_dir: true,
                        });
                    } else {
                        // Check for video extensions
                        let ext = name.rsplit('.').next()
                            .map(|e| e.to_lowercase())
                            .unwrap_or_default();
                        
                        info!("FileBrowser: File '{}' has extension '{}'", name, ext);
                        
                        if matches!(ext.as_str(), "mp4" | "mkv" | "avi" | "webm" | "mov" | "m4v" | "3gp") {
                            files.push(FileEntry {
                                name: format!("🎬 {}", name),
                                path,
                                is_dir: false,
                            });
                        }
                    }
                }
                
                info!("FileBrowser: Found {} dirs, {} videos", dirs.len(), files.len());
                
                // Sort alphabetically
                dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                
                self.entries.extend(dirs);
                self.entries.extend(files);
            }
            Err(e) => {
                error!("FileBrowser: Cannot read directory: {}", e);
                self.error_msg = Some(format!("Cannot access: {}\nGrant storage permission in Settings", e));
            }
        }
    }
    
    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }
    
    /// Move selection down
    pub fn move_down(&mut self) {
        if self.selected_index < self.entries.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }
    
    /// Select current entry (enter folder or pick file)
    pub fn select_current(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index) {
            if entry.is_dir {
                self.current_path = entry.path.clone();
                self.refresh_entries();
            } else {
                // Select the video file
                self.selected_file = Some(entry.path.clone());
                self.visible = false;
            }
        }
    }
    
    /// Go back to parent directory
    pub fn go_back(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            if self.current_path != PathBuf::from("/storage/emulated/0") {
                self.current_path = parent.to_path_buf();
                self.refresh_entries();
            }
        }
    }
    
    /// Take the selected file (if any) - resets after reading
    pub fn take_selected_file(&mut self) -> Option<PathBuf> {
        self.selected_file.take()
    }
}

pub struct VrUi {
    pub params: VrParams,
    pub open: bool,
    pub main_menu_visible: bool,
    pub menu_state: MenuState,
    // Auto-hide hamburger
    pub hamburger_visible: bool,
    last_interaction: Instant,
    // In-VR file browser
    pub file_browser: FileBrowser,
}

impl VrUi {
    pub fn new(ctx: &Context) -> Self {
        Self::apply_material_theme(ctx);
        
        Self {
            params: VrParams::default(),
            open: true,
            main_menu_visible: false,
            menu_state: MenuState::Main,
            hamburger_visible: true,
            last_interaction: Instant::now(),
            file_browser: FileBrowser::new(),
        }
    }

    fn apply_material_theme(ctx: &Context) {
        let mut style = Style::default();
        
        // 1. Spacing - Compact
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 12.0);
        style.spacing.slider_width = 150.0; // Shorter sliders for horizontal layout
        style.spacing.icon_width = 24.0;
        style.spacing.indent = 16.0;
        
        // 2. Visuals - Material Expressive (Dark)
        let mut visuals = Visuals::dark();
        
        // Rounding (Pill shapes)
        visuals.widgets.noninteractive.rounding = Rounding::same(12.0);
        visuals.widgets.inactive.rounding = Rounding::same(12.0);
        visuals.widgets.hovered.rounding = Rounding::same(12.0);
        visuals.widgets.active.rounding = Rounding::same(12.0);
        visuals.widgets.open.rounding = Rounding::same(12.0);
        visuals.window_rounding = Rounding::same(16.0);
        
        // Colors
        let primary = Color32::from_rgb(100, 180, 255); 
        let surface = Color32::from_rgb(30, 30, 35);
        let background = Color32::from_rgb(15, 15, 20);
        
        visuals.panel_fill = surface;
        visuals.window_fill = surface;
        
        // Buttons
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(45, 45, 50);
        visuals.widgets.active.bg_fill = primary;
        visuals.selection.bg_fill = primary;
        
        // Fonts - Compact
        style.text_styles.insert(egui::TextStyle::Body, FontId::new(16.0, FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Button, FontId::new(24.0, FontFamily::Proportional)); // Large icons

        ctx.set_style(style);
        ctx.set_visuals(visuals);
    }
    
    /// Call this on tap to show hamburger and reset auto-hide timer
    pub fn show_hamburger(&mut self) {
        self.hamburger_visible = true;
        self.last_interaction = Instant::now();
    }
    
    /// Toggle hamburger visibility (for gamepad △ button)
    pub fn toggle_hamburger(&mut self) {
        self.hamburger_visible = !self.hamburger_visible;
        self.last_interaction = Instant::now();
    }
    
    /// Check if hamburger menu is visible
    pub fn is_hamburger_visible(&self) -> bool {
        self.hamburger_visible
    }

    pub fn render(&mut self, ctx: &Context, vr_mode_active: bool) {
        // 1. If not in VR mode, do not render anything
        if !vr_mode_active {
            return;
        }
        
        // Auto-hide hamburger after 3 seconds of inactivity
        const AUTO_HIDE_SECONDS: u64 = 3;
        if self.last_interaction.elapsed().as_secs() >= AUTO_HIDE_SECONDS {
            self.hamburger_visible = false;
        }

        // 2. Hamburger Menu (Top-Left, Fixed) - only show if visible
        if self.hamburger_visible {
            egui::Window::new("Hamburger")
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(20.0, 20.0))
                .resizable(false)
                .collapsible(false)
                .title_bar(false)
                .frame(egui::Frame::window(&ctx.style())
                    .inner_margin(Margin::same(8.0))
                    .rounding(Rounding::same(24.0)) 
                    .fill(Color32::from_gray(40)))
                .show(ctx, |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("☰").size(24.0).strong())
                        .frame(false)
                        .min_size(egui::vec2(32.0, 32.0))).clicked() 
                    {
                        self.main_menu_visible = !self.main_menu_visible;
                        self.last_interaction = Instant::now(); // Reset timer on interaction
                        if self.main_menu_visible {
                            self.menu_state = MenuState::Main; // Reset to main when opening
                        }
                    }
                });
        }
        // 3. Main Dock (Bottom Center) - Only if visible
        if self.main_menu_visible {
            // Invisible Close Button (Full Screen)
             egui::Area::new("close_handler".into())
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                     // Fill screen with invisible button to catch clicks
                     let rect = ctx.screen_rect();
                     if ui.allocate_rect(rect, egui::Sense::click()).clicked() {
                         self.main_menu_visible = false;
                     }
                });
        
            let main_dock = egui::Window::new("Dock")
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
                .resizable(false)
                .collapsible(false)
                .title_bar(false)
                .frame(egui::Frame::window(&ctx.style())
                    .inner_margin(Margin::same(12.0))
                    .rounding(Rounding::same(20.0))
                    .fill(Color32::from_black_alpha(220))
                    .stroke(Stroke::new(1.0, Color32::from_gray(60))));

            main_dock.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match self.menu_state {
                        MenuState::Main => {
                            // 1. Recenter
                            if Self::icon_button(ui, "◎", false).clicked() {
                                self.params.recenter_flag = true;
                            }
                            
                            ui.add_space(8.0);
                            
                            // 2. Gyro
                            let gyro_icon = if self.params.gyro_enabled { "🔄" } else { "🛑" };
                            if Self::icon_button(ui, gyro_icon, self.params.gyro_enabled).clicked() {
                                self.params.gyro_enabled = !self.params.gyro_enabled;
                            }

                            ui.add_space(8.0);

                            // 2.5 Video Picker
                             if Self::icon_button(ui, "📂", false).clicked() {
                                self.params.select_video_flag = true;
                            }

                            ui.add_space(8.0);

                            // Playback Controls
                            ui.separator();
                            ui.add_space(8.0);
                            
                            // Seek Backward
                            if Self::icon_button(ui, "⏪", false).clicked() {
                                self.params.seek_backward_flag = true;
                            }
                            
                            ui.add_space(4.0);
                            
                            // Play/Pause
                            if Self::icon_button(ui, "⏯", false).clicked() {
                                self.params.toggle_play_pause = true;
                            }
                            
                            ui.add_space(4.0);
                            
                            // Seek Forward
                            if Self::icon_button(ui, "⏩", false).clicked() {
                                self.params.seek_forward_flag = true;
                            }
                            
                            ui.add_space(8.0);
                            ui.separator();
                            
                            // 3. Settings
                            if Self::icon_button(ui, "⚙", false).clicked() {
                                self.menu_state = MenuState::LensSettings;
                            }
                            
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // 4. Exit
                            if ui.add(egui::Button::new(egui::RichText::new("❌").size(24.0).color(Color32::from_rgb(255, 100, 100)))
                                .min_size(egui::vec2(50.0, 50.0))).clicked() 
                            {
                                self.params.vr_exit_requested = true;
                            }
                        }
                        MenuState::LensSettings => {
                            // 1. Back
                            if Self::icon_button(ui, "⬅", false).clicked() {
                                self.menu_state = MenuState::Main;
                            }
                            
                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(12.0);
                            
                            // 2. Sliders (Horizontal Layout)
                            // Size
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("Size").size(14.0));
                                ui.add(egui::Slider::new(&mut self.params.lens_radius, 0.5..=1.5).fixed_decimals(2));
                            });
                            
                            ui.add_space(12.0);
                            
                            // Distance
                             ui.vertical(|ui| {
                                ui.label(egui::RichText::new("Dist").size(14.0));
                                ui.add(egui::Slider::new(&mut self.params.lens_center_offset, -0.15..=0.15).fixed_decimals(3));
                            });

                             ui.add_space(12.0);
                            
                            // Content Zoom
                             ui.vertical(|ui| {
                                ui.label(egui::RichText::new("Zoom").size(14.0));
                                ui.add(egui::Slider::new(&mut self.params.content_scale, 0.5..=2.0).fixed_decimals(2));
                            });
                        }
                    }
                });
            });
        }
        
        // 4. File Browser Panel (centered, when visible)
        if self.file_browser.visible {
            egui::Window::new("📂 Select Video")
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .resizable(false)
                .collapsible(false)
                .fixed_size(egui::vec2(500.0, 400.0))
                .frame(egui::Frame::window(&ctx.style())
                    .inner_margin(Margin::same(16.0))
                    .rounding(Rounding::same(16.0))
                    .fill(Color32::from_rgba_unmultiplied(20, 20, 25, 240)))
                .show(ctx, |ui| {
                    // Header with current path
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("📁").size(20.0));
                        let path_str = self.file_browser.current_path.to_string_lossy();
                        ui.label(egui::RichText::new(path_str.as_ref()).size(12.0).weak());
                    });
                    ui.separator();
                    
                    // File list with scroll
                    let mut clicked_index: Option<usize> = None;
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (i, entry) in self.file_browser.entries.iter().enumerate() {
                                let is_selected = i == self.file_browser.selected_index;
                                
                                let response = ui.selectable_label(
                                    is_selected,
                                    egui::RichText::new(&entry.name)
                                        .size(18.0)
                                        .color(if is_selected {
                                            Color32::BLACK // Black text for selected
                                        } else if entry.is_dir {
                                            Color32::WHITE
                                        } else {
                                            Color32::from_rgb(200, 200, 200)
                                        })
                                        .background_color(if is_selected {
                                            Color32::from_rgba_unmultiplied(50, 120, 200, 80) // Transparent blue bg
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                );
                                
                                // Auto-scroll to keep selected item visible
                                if is_selected {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                
                                // Track click for handling after loop
                                if response.clicked() {
                                    clicked_index = Some(i);
                                }
                            }
                            
                            if self.file_browser.entries.is_empty() {
                                if let Some(err) = &self.file_browser.error_msg {
                                    ui.label(egui::RichText::new("⚠️ Permission Denied").size(18.0).color(Color32::from_rgb(255, 150, 100)));
                                    ui.label(egui::RichText::new(err).size(14.0).weak());
                                    ui.add_space(10.0);
                                    ui.label(egui::RichText::new("Go to Settings > Apps > VR Space > Permissions").size(12.0).weak());
                                } else {
                                    ui.label(egui::RichText::new("No video files found").size(16.0).weak());
                                    ui.label(egui::RichText::new("Navigate to a folder with .mp4, .mkv files").size(12.0).weak());
                                }
                            }
                        });
                    
                    // Handle click after loop to avoid borrow conflict
                    if let Some(i) = clicked_index {
                        self.file_browser.selected_index = i;
                        self.file_browser.select_current();
                    }
                    
                    ui.separator();
                    
                    // Controls hint
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("X: Select  ○: Back  △: Close").size(12.0).weak());
                    });
                });
        }
    }
    
    fn icon_button(ui: &mut egui::Ui, icon: &str, active: bool) -> egui::Response {
        let btn = egui::Button::new(egui::RichText::new(icon).size(26.0))
            .min_size(egui::vec2(50.0, 50.0))
            .fill(if active { 
                ui.visuals().widgets.active.bg_fill 
            } else { 
                ui.visuals().widgets.inactive.bg_fill 
            });
        ui.add(btn)
    }
}
