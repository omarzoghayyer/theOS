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
pub enum ActiveSurface { Lock, Home, Call, Assistant, Settings, AiShell, Messenger, WebProxy }

/// Visual state of the AI orb -- drives animation and color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrbState {
    /// Idle / sleeping -- slow ambient pulse.
    Passive,
    /// Actively listening to the user -- bright, reactive pulse.
    Listening,
    /// Processing a command -- spinning / thinking animation.
    Processing,
    /// Speaking a response -- waveform-style animation.
    Responding,
}

impl OrbState {
    /// Whether the orb should render in an active (bright) state.
    pub fn is_active(&self) -> bool {
        !matches!(self, OrbState::Passive)
    }

    /// Label for debug output.
    pub fn label(&self) -> &'static str {
        match self {
            OrbState::Passive    => "passive",
            OrbState::Listening  => "listening",
            OrbState::Processing => "processing",
            OrbState::Responding => "responding",
        }
    }
}

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

}

// -- Screen constants for OnePlus 6 ------------------------------------------
pub const SCREEN_W_OP6: i32 = 1080;
pub const SCREEN_H_OP6: i32 = 2280;

// -- ActiveSurface update -----------------------------------------------------
// Note: add Messenger and WebProxy to the existing enum when ready.
// For now they live here as constants for the transition system.
pub const SURFACE_MESSENGER: u8 = 10;
pub const SURFACE_WEB_PROXY: u8 = 11;

// -- Transition state ---------------------------------------------------------
// Drives slide-up animation between surfaces.
// progress: 0.0 = fully off-screen below, 1.0 = fully settled in place.

#[derive(Debug, Clone)]
pub struct TransitionState {
    pub from:       ActiveSurface,
    pub to:         ActiveSurface,
    pub started_at: f64,   // seconds since epoch
    pub duration:   f64,   // seconds
}

impl TransitionState {
    pub fn new(from: ActiveSurface, to: ActiveSurface, now: f64) -> Self {
        Self { from, to, started_at: now, duration: 0.38 }
    }

    /// 0.0 -> 1.0 progress, ease-out cubic.
    pub fn progress(&self, now: f64) -> f64 {
        let t = ((now - self.started_at) / self.duration).clamp(0.0, 1.0);
        // ease-out cubic: fast start, gentle landing
        1.0 - (1.0 - t).powi(3)
    }

    pub fn is_complete(&self, now: f64) -> bool {
        now >= self.started_at + self.duration
    }

    /// Y offset for the incoming surface.
    /// At progress=0: surface starts at screen_h (below screen).
    /// At progress=1: surface is at y=0 (fully visible).
    pub fn incoming_y(&self, screen_h: i32, now: f64) -> i32 {
        let p = self.progress(now);
        (screen_h as f64 * (1.0 - p)) as i32
    }
}

// -- Orb render ---------------------------------------------------------------

impl RenderPipeline {

    /// Draw the full AI orb home screen.
    /// Called every frame when ActiveSurface::AiShell is active.
    /// t = seconds since epoch (f64 for sub-second animation precision).
    pub fn draw_orb_screen(
        &self,
        frame:      &mut GlesFrame,
        t:          f64,
        sat_ok:     bool,
        latency_ms: u32,
        listening:  bool,
    ) {
        // Deep space background
        draw_rect(frame, 0, 0, self.width, self.height, colors::BACKGROUND);

        // Status bar
        self.draw_status_bar(frame, "theOS", 85, sat_ok);

        // Star field -- static seeded dots
        self.draw_star_field_orb(frame);

        // Orb -- vertically centered in upper 60% of screen
        let orb_cx = self.width / 2;
        let orb_cy = STATUS_H + (self.height - STATUS_H) * 2 / 5;
        self.draw_orb(frame, orb_cx, orb_cy, t, listening);

        // Satellite pill below orb
        let pill_y = orb_cy + 160;
        self.draw_sat_pill(frame, sat_ok, latency_ms, pill_y);

        // Prompt text area -- two placeholder bars (text via fontdue Phase 2)
        let prompt_y = pill_y + 60;
        let bar_w    = 320;
        let bar_x    = (self.width - bar_w) / 2;
        // Main prompt line
        draw_rect(frame, bar_x, prompt_y, bar_w, 28, [1.0, 1.0, 1.0, 0.55]);
        // Sub line
        draw_rect(frame, bar_x + 60, prompt_y + 40, bar_w - 120, 18, [1.0, 1.0, 1.0, 0.18]);

        // Hint chips row
        let hints_y = prompt_y + 100;
        self.draw_hint_chips(frame, hints_y);

        // Input bar at bottom
        self.draw_orb_input_bar(frame, t, listening);
    }

    /// Draw the glowing AI orb.
    /// cx, cy = center. t = time. listening = microphone active.
    pub fn draw_orb(
        &self,
        frame:     &mut GlesFrame,
        cx:        i32,
        cy:        i32,
        t:         f64,
        listening: bool,
    ) {
        // -- Outer rings (3 concentric, pulsing with phase offset) --
        let ring_configs: [(i32, f32, f64); 3] = [
            (200, 0.06, 0.0),   // outermost: largest, most transparent, slowest
            (160, 0.10, 0.4),
            (128, 0.16, 0.8),   // innermost ring: brightest
        ];

        for (diameter, base_alpha, phase_offset) in &ring_configs {
            let r       = diameter / 2;
            let pulse   = (((t + phase_offset) * 0.8).sin() * 0.5 + 0.5) as f32;
            let alpha   = base_alpha * (0.6 + 0.4 * pulse);
            let scale   = 1.0 + 0.03 * (((t + phase_offset) * 0.8).sin() as f32);
            let r_scaled = (r as f32 * scale) as i32;

            // Ring = large circle minus slightly smaller circle (hollow)
            let ring_color = [
                colors::ACCENT[0],
                colors::ACCENT[1],
                colors::ACCENT[2],
                alpha,
            ];
            // Draw ring as thin annulus: outer circle
            draw_circle(frame, cx, cy, r_scaled, ring_color);
            // Erase inner part to make it hollow
            let inner_r = r_scaled - 3;
            if inner_r > 0 {
                draw_circle(frame, cx, cy, inner_r, colors::BACKGROUND);
            }
        }

        // -- Orb body --
        let orb_r = 110_i32;

        // Orb glow (slightly larger, dimmer circle behind the orb)
        let glow_pulse = (t * 1.2).sin() as f32 * 0.5 + 0.5;
        let glow_r     = orb_r + 8 + (glow_pulse * 6.0) as i32;
        draw_circle(frame, cx, cy, glow_r, [0.10, 0.25, 0.60, 0.18 + glow_pulse * 0.08]);

        // Core orb -- deep blue
        draw_circle(frame, cx, cy, orb_r, [0.07, 0.15, 0.50, 1.0]);

        // Mid layer -- slightly lighter blue
        draw_circle(frame, cx, cy, (orb_r as f32 * 0.75) as i32, [0.12, 0.25, 0.70, 1.0]);

        // Inner shimmer -- offset highlight to simulate sphere depth
        let shimmer_x = cx - orb_r / 3;
        let shimmer_y = cy - orb_r / 3;
        let shimmer_r = (orb_r as f32 * 0.35) as i32;
        let shimmer_pulse = ((t * 1.8).sin() as f32 * 0.5 + 0.5) * 0.25 + 0.15;
        draw_circle(frame, shimmer_x, shimmer_y, shimmer_r,
            [0.60, 0.75, 1.00, shimmer_pulse]);

        // Listening state: extra bright ring + faster pulse
        if listening {
            let listen_pulse = ((t * 4.0).sin() as f32 * 0.5 + 0.5);
            let listen_r     = orb_r + 20 + (listen_pulse * 12.0) as i32;
            draw_circle(frame, cx, cy, listen_r,
                [colors::ACCENT_GREEN[0], colors::ACCENT_GREEN[1], colors::ACCENT_GREEN[2],
                 0.25 * listen_pulse]);
        }

        println!("[orb] t:{:.2} listening:{} cx:{} cy:{}", t, listening, cx, cy);
    }

    /// Satellite status pill below the orb.
    fn draw_sat_pill(
        &self,
        frame:      &mut GlesFrame,
        sat_ok:     bool,
        latency_ms: u32,
        y:          i32,
    ) {
        let pill_w = 280_i32;
        let pill_h = 52_i32;
        let pill_x = (self.width - pill_w) / 2;

        // Pill background
        draw_rounded_rect(frame, pill_x, y, pill_w, pill_h, pill_h / 2, colors::CARD);

        // Status dot
        let dot_color = if sat_ok { colors::ACCENT_GREEN } else { colors::ACCENT_RED };
        draw_circle(frame, pill_x + 30, y + pill_h / 2, 9, dot_color);

        // Latency bar placeholder
        let lat_bar_w = (pill_w as f32 * 0.45) as i32;
        draw_rect(frame, pill_x + 54, y + 14, lat_bar_w, 14,
            [0.5, 0.6, 0.8, 0.4]);
        draw_rect(frame, pill_x + 54 + lat_bar_w + 12, y + 16, 60, 10,
            [0.4, 0.4, 0.5, 0.3]);

        println!("[orb] sat pill ok:{} {}ms", sat_ok, latency_ms);
    }

    /// Hint chips row -- tappable intent shortcuts.
    fn draw_hint_chips(&self, frame: &mut GlesFrame, y: i32) {
        let chips = ["message Sarah", "open Instagram", "satellite status", "call Marcus"];
        let chip_h  = 56_i32;
        let chip_r  = chip_h / 2;
        let spacing = 16_i32;

        // Two rows of two chips
        for (i, _chip) in chips.iter().enumerate() {
            let row = (i / 2) as i32;
            let col = (i % 2) as i32;
            let chip_w = (self.width - 80 - spacing) / 2;
            let x = 40 + col * (chip_w + spacing);
            let chip_y = y + row * (chip_h + 12);

            // Chip background
            draw_rounded_rect(frame, x, chip_y, chip_w, chip_h, chip_r,
                [0.10, 0.10, 0.16, 1.0]);

            // Chip border (thin accent line on top)
            draw_rect(frame, x + chip_r, chip_y, chip_w - chip_r * 2, 2,
                [0.20, 0.40, 0.80, 0.3]);

            // Text placeholder
            let text_w = chip_w - 32;
            draw_rect(frame, x + 16, chip_y + (chip_h - 18) / 2, text_w, 18,
                [1.0, 1.0, 1.0, 0.30]);
        }

        println!("[orb] hint chips drawn at y:{}", y);
    }

    /// Input bar at the bottom of the orb screen.
    fn draw_orb_input_bar(&self, frame: &mut GlesFrame, t: f64, listening: bool) {
        let bar_h = 140_i32;
        let bar_y = self.height - bar_h;

        // Background fade
        draw_rect(frame, 0, bar_y, self.width, bar_h, [0.03, 0.03, 0.06, 0.96]);

        // Input field
        let field_h = 80_i32;
        let field_y = bar_y + (bar_h - field_h) / 2;
        let field_x = 40_i32;
        let field_w = self.width - 40 - 110;

        draw_rounded_rect(frame, field_x, field_y, field_w, field_h, field_h / 2,
            [0.08, 0.08, 0.13, 1.0]);

        // Border glow when listening
        if listening {
            draw_rounded_rect(frame, field_x - 2, field_y - 2, field_w + 4, field_h + 4,
                field_h / 2 + 2,
                [colors::ACCENT_GREEN[0], colors::ACCENT_GREEN[1], colors::ACCENT_GREEN[2], 0.4]);
        }

        // Cursor blink
        let blink = ((t * 1.2).sin() > 0.0) as i32;
        if blink == 1 {
            draw_rect(frame, field_x + 28, field_y + 20, 3, field_h - 40, colors::ACCENT);
        }

        // Mic/send button
        let btn_cx = self.width - 60;
        let btn_cy = bar_y + bar_h / 2;
        let btn_color = if listening { colors::ACCENT_GREEN } else { colors::ACCENT };
        draw_circle(frame, btn_cx, btn_cy, 44, btn_color);

        // Pulse on mic button when listening
        if listening {
            let p = ((t * 3.0).sin() as f32 * 0.5 + 0.5);
            draw_circle(frame, btn_cx, btn_cy, 44 + (p * 8.0) as i32,
                [colors::ACCENT_GREEN[0], colors::ACCENT_GREEN[1], colors::ACCENT_GREEN[2],
                 0.3 * p]);
        }
    }

    /// Draw the slide-up transition between two surfaces.
    /// incoming_y: y offset of the incoming surface (from TransitionState::incoming_y).
    /// Call this each frame during a transition.
    pub fn draw_surface_transition(
        &self,
        frame:       &mut GlesFrame,
        incoming_y:  i32,
        t:           f64,
        sat_ok:      bool,
        latency_ms:  u32,
    ) {
        // Outgoing surface: orb screen (always transitioning FROM orb for now)
        // Fades out slightly as incoming slides up
        let fade = (incoming_y as f32 / self.height as f32).clamp(0.0, 1.0);
        self.draw_orb_screen(frame, t, sat_ok, latency_ms, false);

        // Incoming surface placeholder -- dark card sliding up from bottom
        if incoming_y < self.height {
            let card_color = [0.08, 0.08, 0.13, 1.0];
            draw_rect(frame, 0, incoming_y, self.width, self.height - incoming_y, card_color);

            // Drag handle at top of incoming surface
            let handle_w = 120_i32;
            let handle_h = 6_i32;
            draw_rounded_rect(
                frame,
                (self.width - handle_w) / 2,
                incoming_y + 16,
                handle_w, handle_h, handle_h / 2,
                [0.30, 0.30, 0.40, 0.5],
            );
        }

        println!("[render] transition incoming_y:{} fade:{:.2}", incoming_y, fade);
    }

    /// Decorative star field for the orb screen background.
    fn draw_star_field_orb(&self, frame: &mut GlesFrame) {
        // 32 deterministic stars seeded by fixed values
        let positions: [(i32, i32, f32); 20] = [
            (80, 180, 0.12), (240, 120, 0.08), (560, 200, 0.10),
            (900, 160, 0.07), (140, 340, 0.09), (700, 300, 0.11),
            (380, 140, 0.06), (820, 420, 0.08), (60, 500, 0.10),
            (960, 240, 0.09), (300, 600, 0.07), (640, 520, 0.12),
            (180, 700, 0.08), (780, 600, 0.10), (440, 800, 0.06),
            (100, 900, 0.09), (860, 800, 0.07), (520, 1000, 0.11),
            (200, 1100, 0.08), (720, 1050, 0.09),
        ];

        for (x, y, brightness) in &positions {
            // Skip stars behind the orb center area
            let orb_cx = self.width / 2;
            let orb_cy = STATUS_H + (self.height - STATUS_H) * 2 / 5;
            let dx = x - orb_cx;
            let dy = y - orb_cy;
            if (dx * dx + dy * dy) < 130 * 130 { continue; }

            draw_circle(frame, *x, *y, 2, [*brightness, *brightness, brightness + 0.02, 0.8]);
        }
    }
}
