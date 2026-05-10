// messenger_ui.rs -- theOS Messenger UI
//
// Renders the conversation screen and conversation list.
// Animations driven by frame timestamp (t: f64, seconds since epoch).
//
// Visual design:
//   - Deep space dark background
//   - Outgoing bubbles: right-aligned, satellite blue gradient
//   - Incoming bubbles: left-aligned, card dark with green accent
//   - Bubble slide-in: ease-out cubic from off-screen edge
//   - Typing indicator: three dots pulsing in wave pattern
//   - Delivery icons: hollow circle -> v -> vv -> VV (blue)
//   - Conversation list: contact avatar circle, preview, unread badge

use crate::render::{
    Surface, TouchState, RenderPipeline,
    draw_rect, draw_rounded_rect, draw_circle,
    colors, STATUS_H, SCREEN_W, SCREEN_H,
};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

// -- Layout constants ---------------------------------------------------------

const BUBBLE_MAX_W:    i32 = 820;  // max bubble width
const BUBBLE_MIN_H:    i32 = 80;   // min bubble height
const BUBBLE_PADDING:  i32 = 32;   // internal padding
const BUBBLE_RADIUS:   i32 = 36;   // corner radius
const BUBBLE_SPACING:  i32 = 16;   // gap between bubbles
const MARGIN:          i32 = 40;   // screen edge margin
const AVATAR_R:        i32 = 40;   // avatar circle radius
const INPUT_H:         i32 = 120;  // input bar height
const HEADER_H:        i32 = 120;  // conversation header height
const UNREAD_BADGE_R:  i32 = 22;   // unread count badge radius

// -- Bubble colors ------------------------------------------------------------
// Outgoing: satellite blue
const BUBBLE_OUT:      [f32; 4] = [0.15, 0.50, 0.95, 1.0];
const BUBBLE_OUT_DARK: [f32; 4] = [0.10, 0.38, 0.78, 1.0]; // shadow/tail
// Incoming: deep card
const BUBBLE_IN:       [f32; 4] = [0.13, 0.13, 0.20, 1.0];
const BUBBLE_IN_GLOW:  [f32; 4] = [0.10, 0.80, 0.45, 0.8]; // green accent
// Read receipt blue
const READ_BLUE:       [f32; 4] = [0.20, 0.60, 1.00, 1.0];
// Typing dot
const TYPING_DOT:      [f32; 4] = [0.40, 0.40, 0.55, 1.0];
const TYPING_DOT_LIT:  [f32; 4] = [0.70, 0.70, 0.90, 1.0];

// -- MessengerView ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MessengerScreen {
    ConversationList,
    Conversation { contact_key_hex: String },
}

pub struct MessengerView {
    pub screen:        MessengerScreen,
    pub scroll_y:      i32,    // scroll offset in conversation
    scroll_velocity:   f32,    // for momentum scrolling
}

impl MessengerView {
    pub fn new() -> Self {
        Self {
            screen:          MessengerScreen::ConversationList,
            scroll_y:        0,
            scroll_velocity: 0.0,
        }
    }

    pub fn open_conversation(&mut self, key_hex: String) {
        self.screen   = MessengerScreen::Conversation { contact_key_hex: key_hex };
        self.scroll_y = 0;
    }

    pub fn go_back(&mut self) {
        self.screen = MessengerScreen::ConversationList;
    }
}

impl Surface for MessengerView {
    fn name(&self) -> &str { "messenger" }

    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!("[messenger] render -- screen: {:?}", self.screen);
    }

    fn handle_touch(&mut self, _x: f64, y: f64, state: TouchState) {
        if state == TouchState::Move {
            // Scroll handled by render pipeline with delta
        }
        // Back gesture: swipe right from left edge
        if state == TouchState::Down && _x < 60.0 {
            if matches!(self.screen, MessengerScreen::Conversation { .. }) {
                self.go_back();
            }
        }
    }
}

// -- RenderPipeline messenger methods -----------------------------------------

impl RenderPipeline {

    // -- Conversation list -----------------------------------------------------

    /// Draw the full conversation list screen.
    pub fn draw_conversation_list(
        &self,
        frame: &mut GlesFrame,
        conversations: &[ConvPreview],
        t: f64,
    ) {
        // Background
        draw_rect(frame, 0, 0, self.width, self.height, colors::BACKGROUND);

        // Header
        self.draw_messenger_header(frame, "Messages", false, t);

        // Conversation rows
        let list_top = STATUS_H + HEADER_H;
        let row_h    = 160_i32;

        for (i, conv) in conversations.iter().enumerate() {
            let y = list_top + i as i32 * row_h;
            if y > self.height { break; }
            self.draw_conversation_row(frame, conv, y, row_h, t);

            // Divider
            draw_rect(
                frame,
                MARGIN + AVATAR_R * 2 + 24, y + row_h - 1,
                self.width - MARGIN * 2 - AVATAR_R * 2 - 24, 1,
                colors::DIVIDER,
            );
        }

        // Empty state
        if conversations.is_empty() {
            println!("[messenger] empty -- no conversations yet");
        }
    }

    /// Draw a single conversation row in the list.
    fn draw_conversation_row(
        &self,
        frame: &mut GlesFrame,
        conv:  &ConvPreview,
        y:     i32,
        h:     i32,
        t:     f64,
    ) {
        let cx = MARGIN + AVATAR_R;
        let cy = y + h / 2;

        // Avatar circle -- color derived from contact name
        let avatar_color = name_color(&conv.contact_name);
        draw_circle(frame, cx, cy, AVATAR_R, avatar_color);

        // Online dot (bottom-right of avatar)
        if conv.online {
            draw_circle(frame, cx + AVATAR_R - 8, cy + AVATAR_R - 8, 12, colors::ACCENT_GREEN);
            draw_circle(frame, cx + AVATAR_R - 8, cy + AVATAR_R - 8, 8,  colors::BACKGROUND);
            draw_circle(frame, cx + AVATAR_R - 8, cy + AVATAR_R - 8, 6,  colors::ACCENT_GREEN);
        }

        // Unread badge
        if conv.unread > 0 {
            draw_circle(frame, cx, y + 16, UNREAD_BADGE_R, colors::ACCENT);
            println!("[messenger] unread badge: {}", conv.unread);
        }

        // Text area
        let text_x = MARGIN + AVATAR_R * 2 + 24;
        let name_y  = y + 40;
        let prev_y  = y + 90;
        let time_x  = self.width - MARGIN - 120;

        // Name bar (placeholder rect -- text rendering via fontdue in Phase 2)
        let name_w = if conv.unread > 0 { 340 } else { 400 };
        draw_rect(frame, text_x, name_y, name_w, 28, colors::TEXT_PRIMARY);

        // Preview bar
        let prev_w = self.width - text_x - MARGIN - 140;
        let prev_color = if conv.unread > 0 { colors::TEXT_PRIMARY } else { colors::TEXT_SECONDARY };
        draw_rect(frame, text_x, prev_y, prev_w, 22, prev_color);

        // Typing indicator in preview position
        if conv.typing {
            self.draw_typing_dots_inline(frame, text_x, prev_y, t);
        }

        // Timestamp
        draw_rect(frame, time_x, name_y, 100, 20, colors::TEXT_SECONDARY);

        println!(
            "[messenger] row: {} unread:{} online:{}",
            conv.contact_name, conv.unread, conv.online
        );
    }

    // -- Conversation screen ---------------------------------------------------

    /// Draw the full conversation screen.
    pub fn draw_conversation(
        &self,
        frame:    &mut GlesFrame,
        contact:  &str,
        messages: &[BubbleData],
        typing:   bool,
        scroll_y: i32,
        t:        f64,
    ) {
        // Background -- subtle gradient feel via layered rects
        draw_rect(frame, 0, 0, self.width, self.height, colors::BACKGROUND);

        // Faint star field dots (static, seeded by contact name)
        self.draw_star_field(frame, contact);

        // Header
        self.draw_messenger_header(frame, contact, true, t);

        // Message area
        let msg_top    = STATUS_H + HEADER_H;
        let msg_bottom = self.height - INPUT_H - 20;
        let msg_h      = msg_bottom - msg_top;

        // Clip region (simulated -- draw background to mask overflow)
        draw_rect(frame, 0, 0, self.width, msg_top, colors::BACKGROUND);

        // Draw bubbles bottom-up
        let mut cursor_y = msg_bottom - scroll_y;

        // Typing indicator (above last message)
        if typing {
            cursor_y -= 80;
            self.draw_typing_indicator(frame, cursor_y, t);
            cursor_y -= BUBBLE_SPACING;
        }

        // Bubbles in reverse (newest at bottom)
        for bubble in messages.iter().rev() {
            let h = bubble_height(bubble);
            cursor_y -= h + BUBBLE_SPACING;

            if cursor_y > msg_bottom { continue; }  // below viewport
            if cursor_y + h < msg_top { break; }    // above viewport

            self.draw_bubble(frame, bubble, cursor_y, t);
        }

        // Input bar
        self.draw_messenger_input(frame, t);

        // Fade overlay at top of message area (depth cue)
        for i in 0..40 {
            let alpha = (40 - i) as f32 / 40.0 * 0.6;
            draw_rect(
                frame, 0, msg_top + i, self.width, 1,
                [0.05, 0.05, 0.08, alpha],
            );
        }
    }

    /// Draw a single chat bubble with slide-in animation.
    fn draw_bubble(
        &self,
        frame:  &mut GlesFrame,
        bubble: &BubbleData,
        y:      i32,
        t:      f64,
    ) {
        let h = bubble_height(bubble);
        let w = bubble_width(bubble, self.width);

        // Slide-in animation: bubbles slide from their respective edge
        let anim_progress = ease_out_cubic(
            ((t - bubble.appeared_at) * 1000.0 / 280.0).clamp(0.0, 1.0)
        );

        let x = if bubble.from_me {
            // Right-aligned -- slides in from right
            let final_x = self.width - MARGIN - w;
            let start_x = self.width + 40; // off-screen right
            lerp(start_x as f64, final_x as f64, anim_progress) as i32
        } else {
            // Left-aligned -- slides in from left
            let final_x = MARGIN;
            let start_x = -w - 40; // off-screen left
            lerp(start_x as f64, final_x as f64, anim_progress) as i32
        };

        // Subtle shadow (offset rect, slightly transparent)
        if bubble.from_me {
            draw_rounded_rect(frame, x + 4, y + 4, w, h, BUBBLE_RADIUS, [0.0, 0.0, 0.0, 0.3]);
        }

        // Bubble background
        let bg_color = if bubble.from_me { BUBBLE_OUT } else { BUBBLE_IN };
        draw_rounded_rect(frame, x, y, w, h, BUBBLE_RADIUS, bg_color);

        // Outgoing: accent gradient strip on right edge
        if bubble.from_me {
            draw_rect(frame, x + w - 6, y + BUBBLE_RADIUS, 6, h - BUBBLE_RADIUS * 2, BUBBLE_OUT_DARK);
        }

        // Incoming: green glow strip on left edge
        if !bubble.from_me {
            draw_rect(frame, x, y + BUBBLE_RADIUS, 4, h - BUBBLE_RADIUS * 2, BUBBLE_IN_GLOW);
        }

        // Bubble tail (small triangle at corner)
        if bubble.from_me {
            // Right tail
            draw_rect(frame, x + w, y + h - 40, 16, 24, BUBBLE_OUT);
            draw_rect(frame, x + w + 8, y + h - 32, 12, 16, colors::BACKGROUND);
        } else {
            // Left tail
            draw_rect(frame, x - 16, y + h - 40, 16, 24, BUBBLE_IN);
            draw_rect(frame, x - 20, y + h - 32, 12, 16, colors::BACKGROUND);
        }

        // Message text placeholder (fontdue text rendering in Phase 2)
        let text_x = x + BUBBLE_PADDING;
        let text_y = y + BUBBLE_PADDING;
        let text_w = w - BUBBLE_PADDING * 2;
        draw_rect(frame, text_x, text_y, text_w, 28, [1.0, 1.0, 1.0, 0.9]);
        if bubble.text.len() > 40 {
            draw_rect(frame, text_x, text_y + 36, (text_w as f32 * 0.7) as i32, 28, [1.0, 1.0, 1.0, 0.9]);
        }

        // Timestamp + delivery icon (outgoing only)
        if bubble.from_me {
            let icon_x = x + w - BUBBLE_PADDING - 60;
            let icon_y = y + h - 36;

            // Delivery state color
            let icon_color = match bubble.state {
                DeliveryStateUi::Read    => READ_BLUE,
                DeliveryStateUi::Failed  => colors::ACCENT_RED,
                _                        => colors::TEXT_SECONDARY,
            };

            // Delivery dot/check indicator
            match bubble.state {
                DeliveryStateUi::Sending   => {
                    // Pulsing hollow circle
                    let pulse = ((t * 2.0).sin() * 0.5 + 0.5) as f32;
                    let c = [0.6, 0.6, 0.7, pulse];
                    draw_circle(frame, icon_x + 8, icon_y + 8, 8, c);
                }
                DeliveryStateUi::Sent      => {
                    draw_rect(frame, icon_x, icon_y + 4, 20, 4, icon_color);
                }
                DeliveryStateUi::Delivered => {
                    draw_rect(frame, icon_x,      icon_y + 4, 20, 4, icon_color);
                    draw_rect(frame, icon_x + 8,  icon_y + 8, 20, 4, icon_color);
                }
                DeliveryStateUi::Read      => {
                    // Blue double check -- filled
                    draw_rect(frame, icon_x,      icon_y + 4, 20, 5, icon_color);
                    draw_rect(frame, icon_x + 8,  icon_y + 8, 20, 5, icon_color);
                }
                DeliveryStateUi::Failed    => {
                    draw_circle(frame, icon_x + 8, icon_y + 8, 8, colors::ACCENT_RED);
                }
            }

            // Timestamp bar
            draw_rect(frame, icon_x - 80, icon_y + 4, 72, 18, [0.6, 0.6, 0.7, 0.5]);
        }

        println!(
            "[messenger] bubble from_me:{} anim:{:.2} state:{:?}",
            bubble.from_me, anim_progress, bubble.state
        );
    }

    // -- Typing indicator ------------------------------------------------------

    /// Three animated dots in a bubble -- wave pattern.
    fn draw_typing_indicator(&self, frame: &mut GlesFrame, y: i32, t: f64) {
        let w  = 160_i32;
        let h  = 80_i32;
        let x  = MARGIN;

        // Bubble background
        draw_rounded_rect(frame, x, y, w, h, h / 2, BUBBLE_IN);
        draw_rect(frame, x, y + h / 2, 4, h / 2, BUBBLE_IN_GLOW);

        // Three dots with wave animation
        let dot_r    = 10_i32;
        let dot_spacing = 44_i32;
        let base_y   = y + h / 2;

        for i in 0..3 {
            let offset   = i as f64 * 0.2;
            let cycle    = 0.8_f64;
            let phase    = ((t - offset) % cycle) / cycle;
            let bounce   = (phase * std::f64::consts::TAU).sin();
            let dot_y    = base_y + (bounce * -14.0) as i32;
            let dot_x    = x + 36 + i * dot_spacing;

            // Brightness varies with phase
            let brightness = (bounce * 0.5 + 0.5) as f32;
            let color = [
                TYPING_DOT[0] + (TYPING_DOT_LIT[0] - TYPING_DOT[0]) * brightness,
                TYPING_DOT[1] + (TYPING_DOT_LIT[1] - TYPING_DOT[1]) * brightness,
                TYPING_DOT[2] + (TYPING_DOT_LIT[2] - TYPING_DOT[2]) * brightness,
                1.0,
            ];

            draw_circle(frame, dot_x, dot_y, dot_r, color);
        }

        println!("[messenger] typing indicator t:{:.2}", t);
    }

    /// Inline typing dots for conversation list preview.
    fn draw_typing_dots_inline(&self, frame: &mut GlesFrame, x: i32, y: i32, t: f64) {
        for i in 0..3 {
            let offset = i as f64 * 0.15;
            let phase  = ((t - offset) % 0.6) / 0.6;
            let bright = (phase * std::f64::consts::TAU).sin() * 0.5 + 0.5;
            let color  = [0.5, 0.5, 0.65, bright as f32];
            draw_circle(frame, x + 12 + i * 24, y + 11, 7, color);
        }
    }

    // -- Header ----------------------------------------------------------------

    fn draw_messenger_header(&self, frame: &mut GlesFrame, title: &str, show_back: bool, t: f64) {
        let h = STATUS_H + HEADER_H;

        // Header background with subtle bottom border
        draw_rect(frame, 0, 0, self.width, h, colors::STATUS_BAR);
        draw_rect(frame, 0, h - 2, self.width, 2, colors::DIVIDER);

        // Back arrow (conversation screen only)
        if show_back {
            let arr_x = 40;
            let arr_y = STATUS_H + HEADER_H / 2;
            draw_rect(frame, arr_x,      arr_y - 2,  36, 4, colors::ACCENT);
            draw_rect(frame, arr_x,      arr_y - 14, 4,  16, colors::ACCENT);
            draw_rect(frame, arr_x,      arr_y + 2,  4,  16, colors::ACCENT);

            // Contact avatar in header
            let avatar_color = name_color(title);
            draw_circle(frame, self.width / 2, STATUS_H + HEADER_H / 2, 36, avatar_color);

            // Online indicator on header avatar
            draw_circle(frame, self.width / 2 + 26, STATUS_H + HEADER_H / 2 + 26, 12, colors::ACCENT_GREEN);
        }

        // Title placeholder
        let title_w = (title.len() as i32 * 22).min(400);
        let title_x = (self.width - title_w) / 2;
        let title_y = STATUS_H + HEADER_H / 2 - 14;
        if !show_back {
            draw_rect(frame, title_x, title_y, title_w, 28, colors::TEXT_PRIMARY);
        }

        println!("[messenger] header: {} back:{}", title, show_back);
    }

    // -- Input bar -------------------------------------------------------------

    fn draw_messenger_input(&self, frame: &mut GlesFrame, t: f64) {
        let bar_y = self.height - INPUT_H;

        // Bar background
        draw_rect(frame, 0, bar_y, self.width, INPUT_H, colors::STATUS_BAR);
        draw_rect(frame, 0, bar_y, self.width, 1, colors::DIVIDER);

        // Input field
        let field_x = MARGIN;
        let field_y = bar_y + 20;
        let field_w = self.width - MARGIN * 2 - 100;
        let field_h = INPUT_H - 40;
        draw_rounded_rect(frame, field_x, field_y, field_w, field_h, field_h / 2, colors::CARD);

        // Animated cursor blink
        let blink = ((t * 1.2).sin() > 0.0) as i32;
        if blink == 1 {
            draw_rect(frame, field_x + 24, field_y + 16, 3, field_h - 32, colors::ACCENT);
        }

        // Send button -- glows when input has content
        let btn_cx = self.width - MARGIN - 36;
        let btn_cy = bar_y + INPUT_H / 2;
        draw_circle(frame, btn_cx, btn_cy, 38, colors::ACCENT);

        // Send arrow inside button
        draw_rect(frame, btn_cx - 10, btn_cy - 2, 20, 4, [1.0, 1.0, 1.0, 1.0]);
        draw_rect(frame, btn_cx + 4,  btn_cy - 10, 4, 12, [1.0, 1.0, 1.0, 1.0]);

        println!("[messenger] input bar t:{:.2}", t);
    }

    // -- Star field ------------------------------------------------------------

    /// Faint decorative star dots in message background.
    /// Seeded by contact name so each conversation has a unique sky.
    fn draw_star_field(&self, frame: &mut GlesFrame, seed: &str) {
        let mut hash: u32 = 0x811c9dc5;
        for b in seed.bytes() {
            hash ^= b as u32;
            hash = hash.wrapping_mul(0x01000193);
        }

        for i in 0..24 {
            let h  = hash.wrapping_mul(i * 0x9e3779b9 + 1);
            let x  = (h & 0xFFFF) as i32 % self.width;
            let y  = ((h >> 16) & 0xFFFF) as i32 % self.height;
            let br = ((h >> 8) & 0x3F) as f32 / 255.0 + 0.02;
            draw_circle(frame, x, y, 2, [br, br, br + 0.02, 0.4]);
        }
    }
}

// -- Data types passed to render ----------------------------------------------

/// Lightweight conversation preview for the list screen.
#[derive(Debug, Clone)]
pub struct ConvPreview {
    pub contact_name: String,
    pub preview_text: String,
    pub unread:       usize,
    pub online:       bool,
    pub typing:       bool,
    pub timestamp:    u64,
}

/// Bubble data passed to the renderer each frame.
#[derive(Debug, Clone)]
pub struct BubbleData {
    pub text:        String,
    pub from_me:     bool,
    pub state:       DeliveryStateUi,
    pub appeared_at: f64,   // when this bubble first appeared (for animation)
    pub timestamp:   u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryStateUi {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

// -- Animation helpers --------------------------------------------------------

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn bubble_height(bubble: &BubbleData) -> i32 {
    let lines = (bubble.text.len() as i32 / 38).max(1);
    BUBBLE_MIN_H + (lines - 1) * 40 + BUBBLE_PADDING
}

fn bubble_width(bubble: &BubbleData, screen_w: i32) -> i32 {
    let text_w = (bubble.text.len() as i32 * 18).min(BUBBLE_MAX_W - BUBBLE_PADDING * 2);
    (text_w + BUBBLE_PADDING * 2).max(160).min(screen_w - MARGIN * 2)
}

/// Deterministic color from a name string -- each contact gets their own color.
fn name_color(name: &str) -> [f32; 4] {
    let mut hash: u32 = 0;
    for b in name.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    let hue = (hash % 360) as f32 / 360.0;
    // Convert HSL (hue, 0.5, 0.4) to RGB -- vivid but not blinding
    let c   = 0.5_f32;
    let x   = c * (1.0 - ((hue * 6.0) % 2.0 - 1.0).abs());
    let m   = 0.15_f32;
    let (r, g, b) = if hue < 1.0/6.0      { (c, x, 0.0) }
                    else if hue < 2.0/6.0  { (x, c, 0.0) }
                    else if hue < 3.0/6.0  { (0.0, c, x) }
                    else if hue < 4.0/6.0  { (0.0, x, c) }
                    else if hue < 5.0/6.0  { (x, 0.0, c) }
                    else                   { (c, 0.0, x) };
    [r + m, g + m, b + m, 1.0]
}
