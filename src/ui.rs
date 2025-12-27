use egui::{Context, Visuals, Style, Rounding, Color32, Margin, Stroke, Shadow, FontId, FontFamily};

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
        }
    }
}

pub enum MenuState {
    Main,
    LensSettings,
}

pub struct VrUi {
    pub params: VrParams,
    pub open: bool,
    pub main_menu_visible: bool,
    pub menu_state: MenuState,
}

impl VrUi {
    pub fn new(ctx: &Context) -> Self {
        Self::apply_material_theme(ctx);
        
        Self {
            params: VrParams::default(),
            open: true,
            main_menu_visible: false,
            menu_state: MenuState::Main,
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

    pub fn render(&mut self, ctx: &Context, vr_mode_active: bool) {
        // 1. If not in VR mode, do not render anything
        if !vr_mode_active {
            return;
        }

        // 2. Hamburger Menu (Top-Left, Fixed)
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
                    if self.main_menu_visible {
                        self.menu_state = MenuState::Main; // Reset to main when opening
                    }
                }
            });

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

                            // 2.5 Video Picker
                             if Self::icon_button(ui, "📂", false).clicked() {
                                self.params.select_video_flag = true;
                            }

                            ui.add_space(8.0);
                            
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
