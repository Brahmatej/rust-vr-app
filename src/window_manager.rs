//! Window Manager module
//!
//! Manages floating 3D panels/windows in the VR space.

use glam::{Vec3, Quat, Mat4};

/// A floating window/panel in 3D space
pub struct Panel {
    pub id: u32,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub title: String,
    pub content_type: PanelContent,
}

/// What type of content the panel displays
pub enum PanelContent {
    /// Embedded web browser
    Browser { url: String },
    /// App launcher dock
    Dock,
    /// Settings menu
    Settings,
}

/// Manages all panels in the scene
pub struct WindowManager {
    panels: Vec<Panel>,
    next_id: u32,
    focused_panel: Option<u32>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            panels: Vec::new(),
            next_id: 0,
            focused_panel: None,
        }
    }
    
    /// Spawn a new browser panel
    pub fn spawn_browser(&mut self, url: &str, position: Vec3) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        
        let panel = Panel {
            id,
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(1.6, 0.9, 0.01), // 16:9 aspect ratio
            title: format!("Browser {}", id),
            content_type: PanelContent::Browser { url: url.to_string() },
        };
        
        self.panels.push(panel);
        self.focused_panel = Some(id);
        id
    }
    
    /// Spawn the app dock
    pub fn spawn_dock(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        
        let panel = Panel {
            id,
            position: Vec3::new(0.0, -0.8, -2.0),
            rotation: Quat::from_rotation_x(-0.2), // Slightly tilted up
            scale: Vec3::new(2.0, 0.3, 0.01),
            title: "Dock".to_string(),
            content_type: PanelContent::Dock,
        };
        
        self.panels.push(panel);
        id
    }
    
    /// Move a panel in 3D space
    pub fn move_panel(&mut self, id: u32, delta: Vec3) {
        if let Some(panel) = self.panels.iter_mut().find(|p| p.id == id) {
            panel.position += delta;
        }
    }
    
    /// Scale a panel
    pub fn scale_panel(&mut self, id: u32, scale_factor: f32) {
        if let Some(panel) = self.panels.iter_mut().find(|p| p.id == id) {
            panel.scale *= scale_factor;
            // Clamp scale
            panel.scale = panel.scale.clamp(Vec3::splat(0.3), Vec3::splat(3.0));
        }
    }
    
    /// Close a panel
    pub fn close_panel(&mut self, id: u32) {
        self.panels.retain(|p| p.id != id);
        if self.focused_panel == Some(id) {
            self.focused_panel = self.panels.first().map(|p| p.id);
        }
    }
    
    /// Get model matrix for a panel
    pub fn get_transform(&self, id: u32) -> Option<Mat4> {
        self.panels.iter().find(|p| p.id == id).map(|panel| {
            Mat4::from_scale_rotation_translation(
                panel.scale,
                panel.rotation,
                panel.position,
            )
        })
    }
    
    /// Iterate over all panels
    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }
}
