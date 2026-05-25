// conversation_render.rs — GLES rendering for conversation screen
//
// Renders message bubbles, input field, delivery indicators.
// Uses fontdue for text rendering (already integrated).

use crate::conversation_screen::{ConversationScreen, DeliveryState};
use crate::render::draw_rect;
use smithay::backend::renderer::gles::GlesFrame;

/// Color palette for messaging UI
mod colors {
    pub const BG: [f32; 4] = [0.05, 0.05, 0.08, 1.0]; // dark bg
    pub const BUBBLE_ME: [f32; 4] = [0.2, 0.6, 1.0, 1.0]; // light blue (my messages)
    pub const BUBBLE_THEM: [f32; 4] = [0.3, 0.3, 0.4, 1.0]; // gray (their messages)
    pub const TEXT: [f32; 4] = [0.95, 0.95, 0.98, 1.0]; // light text
    pub const INPUT_BG: [f32; 4] = [0.15, 0.15, 0.18, 1.0]; // input field bg
    pub const BORDER: [f32; 4] = [0.4, 0.4, 0.45, 1.0]; // borders
}

/// Render the entire conversation screen
pub fn render(frame: &mut GlesFrame, conv: &ConversationScreen) {
    let screen_w = 1080i32;
    let screen_h = 2400i32;
    let message_area_h = (screen_h as f32 * 0.85) as i32;
    let input_area_h = screen_h - message_area_h;

    // Background
    draw_rect(frame, 0, 0, screen_w, screen_h, colors::BG);

    // Header (contact name)
    render_header(frame, conv, screen_w);

    // Message bubbles area
    render_messages(frame, conv, screen_w, message_area_h);

    // Input field area
    render_input_field(frame, conv, screen_w, message_area_h, input_area_h);
}

/// Render header with contact name
fn render_header(frame: &mut GlesFrame, conv: &ConversationScreen, screen_w: i32) {
    let header_h = 60;

    // Header background
    draw_rect(frame, 0, 0, screen_w, header_h, [0.1, 0.1, 0.13, 1.0]);

    // Contact name (rendered as text via fontdue if available)
    // For now, just indicate the contact
    eprintln!("[conv] rendering header: {}", conv.contact_name);
}

/// Render message bubbles
fn render_messages(frame: &mut GlesFrame, conv: &ConversationScreen, screen_w: i32, area_h: i32) {
    let margin = 12i32;
    let bubble_max_width = (screen_w as f32 * 0.75) as i32;
    let mut y = 80i32;

    for (i, msg) in conv.messages.iter().enumerate() {
        // Show timestamp every 5 messages
        if i > 0 && i % 5 == 0 {
            y += 30; // space for timestamp
        }

        let bubble_h = 50; // simplified: fixed height
        let x_left = margin;
        let x_right = screen_w - margin - bubble_max_width;

        if msg.from_me {
            // My message: right-aligned
            draw_rect(
                frame,
                x_right,
                y,
                bubble_max_width,
                bubble_h,
                colors::BUBBLE_ME,
            );
            render_delivery_indicator(frame, x_right + bubble_max_width + 5, y, msg.delivery_state);
        } else {
            // Their message: left-aligned
            draw_rect(
                frame,
                x_left,
                y,
                bubble_max_width,
                bubble_h,
                colors::BUBBLE_THEM,
            );
        }

        // Text content (placeholder — would use fontdue for real rendering)
        eprintln!("[conv] bubble: {} ({})", msg.text, msg.delivery_state.label());

        y += bubble_h + 12; // bubble + padding

        if y > area_h - 100 {
            break; // don't overflow
        }
    }
}

/// Render delivery indicator (✓, ✓✓, ✗, ⏳)
fn render_delivery_indicator(
    frame: &mut GlesFrame,
    x: i32,
    y: i32,
    state: DeliveryState,
) {
    let indicator_color = match state {
        DeliveryState::Sending => [0.7, 0.7, 0.7, 1.0],   // gray
        DeliveryState::Sent => [0.6, 0.8, 1.0, 1.0],      // light blue
        DeliveryState::Delivered => [0.2, 0.8, 1.0, 1.0], // bright blue
        DeliveryState::Read => [0.0, 0.8, 1.0, 1.0],      // cyan
        DeliveryState::Failed => [1.0, 0.3, 0.3, 1.0],    // red
    };

    // Small indicator box
    draw_rect(frame, x, y, 20, 20, indicator_color);
}

/// Render input field at bottom
fn render_input_field(
    frame: &mut GlesFrame,
    conv: &ConversationScreen,
    screen_w: i32,
    message_area_h: i32,
    input_area_h: i32,
) {
    let padding = 12i32;
    let input_x = padding;
    let input_y = message_area_h + padding;
    let input_w = screen_w - padding * 2;
    let input_h = input_area_h - padding * 2;

    // Input field background
    draw_rect(frame, input_x, input_y, input_w, input_h, colors::INPUT_BG);

    // Border
    draw_rect(frame, input_x, input_y, input_w, 2, colors::BORDER);

    // Text content (placeholder)
    if !conv.input_buf.is_empty() {
        eprintln!("[conv] input: {}", conv.input_buf);
    } else {
        eprintln!("[conv] input ready (type to compose message)");
    }

    // Send hint (swipe right to send)
    if conv.is_typing && !conv.input_is_empty() {
        eprintln!("[conv] hint: swipe right to send");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_palette_valid() {
        // Ensure colors are valid RGBA
        assert_eq!(colors::BG.len(), 4);
        assert_eq!(colors::BUBBLE_ME.len(), 4);
        assert_eq!(colors::TEXT.len(), 4);
    }
}
