// render.rs — theOS OpenGL ES Render Pipeline
// Draws directly to the Pixel 6 display via GLES + DRM/KMS
// No abstraction layer — pure GPU rendering

use smithay::backend::renderer::gles::{GlesRenderer, GlesFrame, GlesError};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::{Frame, Renderer};
use smithay::utils::{Physical, Rectangle, Size, Transform};

// ── theOS Color Palette ──
// Every color in the OS defined here
pub mod colors {
    pub const BACKGROUND:     [f32; 4] = [0.05, 0.05, 0.08, 1.0]; // near black
    pub const STATUS_BAR:     [f32; 4] = [0.08, 0.08, 0.12, 1.0]; // dark bar
    pub const CARD:           [f32; 4] = [0.10, 0.10, 0.15, 1.0]; // card bg
    pub const ACCENT:         [f32; 4] = [0.20, 0.60, 1.00, 1.0]; // satellite blue
    pub const ACCENT_GREEN:   [f32; 4] = [0.10, 0.90, 0.50, 1.0]; // connected green
    pub const ACCENT_RED:     [f32; 4] = [1.00, 0.25, 0.25, 1.0]; // error red
    pub const TEXT_PRIMARY:   [f32; 4] = [1.00, 1.00, 1.00, 1.0]; // white
    pub const TEXT_SECONDARY: [f32; 4] = [0.60, 0.60, 0.70, 1.0]; // gray
    pub const DIVIDER:        [f32; 4] = [0.15, 0.15, 0.20, 1.0]; // subtle line
    pub const DIALER_KEY:     [f32; 4] = [0.12, 0.12, 0.18, 1.0]; // key background
    pub const CALL_GREEN:     [f32; 4] = [0.10, 0.85, 0.40, 1.0]; // call button
    pub const HANGUP_RED:     [f32; 4] = [0.90, 0.15, 0.15, 1.0]; // hangup button
}

// ── Screen constants for Pixel 6 ──
pub const SCREEN_W: i32 = 1080;
pub const SCREEN_H: i32 = 2400;
pub const STATUS_H: i32 = 100;  // status bar height
pub const DOCK_H:   i32 = 160;  // bottom dock height

// ── Surface trait ──
pub trait Surface {
    fn draw(&self, frame: &mut GlesFrame, size: Size<i32, Physical>);
    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState);
    fn name(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchState { Down, Up, Move }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveSurface { Lock, Home, Dialer, Assistant, Settings, AiShell }

// ── Render helpers ──
// These wrap the smithay GLES frame to make drawing easier

/// Draw a solid colored rectangle
pub fn draw_rect(
    frame: &mut GlesFrame,
    x: i32, y: i32,
    w: i32, h: i32,
    color: [f32; 4],
) {
    let rect = Rectangle::from_loc_and_size((x, y), (w, h));
    frame.clear(Color32F::from(color), &[rect]).ok();
}

/// Draw a rounded rectangle (approximated with overlapping rects)
pub fn draw_rounded_rect(
    frame: &mut GlesFrame,
    x: i32, y: i32,
    w: i32, h: i32,
    radius: i32,
    color: [f32; 4],
) {
    // Main body
    draw_rect(frame, x + radius, y, w - radius*2, h, color);
    // Left and right strips
    draw_rect(frame, x, y + radius, radius, h - radius*2, color);
    draw_rect(frame, x + w - radius, y + radius, radius, h - radius*2, color);
    // Corners (approximated)
    draw_rect(frame, x, y + radius/2, radius, radius, color);
    draw_rect(frame, x + w - radius, y + radius/2, radius, radius, color);
    draw_rect(frame, x, y + h - radius, radius, radius, color);
    draw_rect(frame, x + w - radius, y + h - radius, radius, radius, color);
}

/// Draw a circle (approximated with concentric rects)
pub fn draw_circle(
    frame: &mut GlesFrame,
    cx: i32, cy: i32,
    radius: i32,
    color: [f32; 4],
) {
    let r = radius as f32;
    for dy in -radius..=radius {
        let dx = ((r*r - (dy*dy) as f32).sqrt()) as i32;
        draw_rect(frame, cx - dx, cy + dy, dx * 2, 1, color);
    }
}

// ── Render Pipeline ──
pub struct RenderPipeline {
    pub width: i32,
    pub height: i32,
    pub active_surface: ActiveSurface,
}

impl RenderPipeline {
    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height, active_surface: ActiveSurface::Lock }
    }

    pub fn size(&self) -> Size<i32, Physical> {
        Size::from((self.width, self.height))
    }

    pub fn navigate(&mut self, surface: ActiveSurface) {
        println!("[render] → {:?}", surface);
        self.active_surface = surface;
    }

    /// Draw the status bar — shown on every screen
    pub fn draw_status_bar(
        &self,
        frame: &mut GlesFrame,
        time_str: &str,
        battery_pct: u8,
        satellite_connected: bool,
    ) {
        // Status bar background
        draw_rect(frame, 0, 0, self.width, STATUS_H, colors::STATUS_BAR);

        // Satellite indicator — left side
        let sat_color = if satellite_connected {
            colors::ACCENT_GREEN
        } else {
            colors::ACCENT_RED
        };
        draw_circle(frame, 60, STATUS_H/2, 12, sat_color);

        // Battery indicator — right side
        let bat_w = 60;
        let bat_h = 28;
        let bat_x = self.width - bat_w - 30;
        let bat_y = (STATUS_H - bat_h) / 2;

        // Battery outline
        draw_rounded_rect(frame, bat_x, bat_y, bat_w, bat_h, 4, colors::TEXT_SECONDARY);

        // Battery fill
        let fill_w = ((bat_w - 4) as f32 * battery_pct as f32 / 100.0) as i32;
        let fill_color = if battery_pct > 20 { colors::ACCENT_GREEN } else { colors::ACCENT_RED };
        draw_rect(frame, bat_x + 2, bat_y + 2, fill_w, bat_h - 4, fill_color);

        println!("[render] status bar — time: {} bat: {}% sat: {}",
            time_str, battery_pct, satellite_connected);
    }

    /// Draw the satellite connectivity card on home screen
    pub fn draw_satellite_card(
        &self,
        frame: &mut GlesFrame,
        link: &str,
        latency_ms: u32,
        quality: u8,
    ) {
        let card_x = 40;
        let card_y = STATUS_H + 40;
        let card_w = self.width - 80;
        let card_h = 200;

        // Card background
        draw_rounded_rect(frame, card_x, card_y, card_w, card_h, 20, colors::CARD);

        // Accent line on left edge
        draw_rect(frame, card_x, card_y + 20, 4, card_h - 40, colors::ACCENT);

        // Signal quality bar
        let bar_x = card_x + 30;
        let bar_y = card_y + card_h - 40;
        let bar_w = card_w - 60;
        let bar_h = 8;

        // Background bar
        draw_rect(frame, bar_x, bar_y, bar_w, bar_h, colors::DIVIDER);

        // Quality fill
        let fill_w = (bar_w as f32 * quality as f32 / 100.0) as i32;
        draw_rect(frame, bar_x, bar_y, fill_w, bar_h, colors::ACCENT);

        println!("[render] satellite card — {} {}ms quality:{}%",
            link, latency_ms, quality);
    }

    /// Draw the app grid on home screen
    pub fn draw_app_grid(&self, frame: &mut GlesFrame, apps: &[&str]) {
        let cols = 4;
        let margin = 40;
        let spacing = 20;
        let cell_w = (self.width - margin*2 - spacing*(cols-1)) / cols;
        let cell_h = cell_w;
        let grid_top = STATUS_H + 300;

        for (i, app) in apps.iter().enumerate() {
            let col = (i % cols as usize) as i32;
            let row = (i / cols as usize) as i32;
            let x = margin + col * (cell_w + spacing);
            let y = grid_top + row * (cell_h + spacing);

            // App icon background
            draw_rounded_rect(frame, x, y, cell_w, cell_h, 20, colors::CARD);

            // Accent dot for first app (Phone)
            if i == 0 {
                draw_circle(frame, x + cell_w/2, y + cell_h/2, 30, colors::ACCENT);
            }

            println!("[render] app icon: {} at ({},{})", app, x, y);
        }
    }

    /// Draw the dialer keypad
    pub fn draw_dialer_keypad(&self, frame: &mut GlesFrame, number: &str) {
        // Number display area
        let display_y = STATUS_H + 200;
        let display_h = 120;
        draw_rect(frame, 0, display_y, self.width, display_h, colors::BACKGROUND);

        // Keypad grid — 3x4
        let key_cols = 3;
        let key_rows = 4;
        let margin = 60;
        let spacing = 20;
        let key_w = (self.width - margin*2 - spacing*(key_cols-1)) / key_cols;
        let key_h = 140;
        let keypad_top = display_y + display_h + 60;

        let keys = [
            "1","2","3",
            "4","5","6",
            "7","8","9",
            "*","0","#",
        ];

        for (i, key) in keys.iter().enumerate() {
            let col = (i % key_cols as usize) as i32;
            let row = (i / key_cols as usize) as i32;
            let x = margin + col * (key_w + spacing);
            let y = keypad_top + row * (key_h + spacing);

            draw_rounded_rect(frame, x, y, key_w, key_h, key_h/2, colors::DIALER_KEY);
            println!("[render] dialer key: {} at ({},{})", key, x, y);
        }

        // Call button
        let btn_r = 80;
        let btn_cx = self.width / 2;
        let btn_cy = keypad_top + key_rows * (key_h + spacing) + 60;
        draw_circle(frame, btn_cx, btn_cy, btn_r, colors::CALL_GREEN);

        println!("[render] dialer — number: '{}'", number);
    }
}
