// call_ui_render.rs — Actual GLES rendering for the call surface
//
// Takes a CallSurface and renders it with text using fontdue.

use crate::call_ui::CallSurface;
use crate::render::{
    draw_rect, draw_rounded_rect, draw_circle, colors,
    SCREEN_W, SCREEN_H, STATUS_H,
};
use crate::text_renderer;
use smithay::backend::renderer::gles::GlesFrame;

const AVATAR_R: i32 = 80;           // contact avatar circle radius
const HEADER_MARGIN: i32 = 60;      // margin from top
const CONTROL_ROW_H: i32 = 160;     // control buttons height
const BUTTON_R: i32 = 60;           // button radius
const FONT_SIZE_LARGE: f32 = 48.0;  // contact name
const FONT_SIZE_MEDIUM: f32 = 36.0; // status/encryption
const FONT_SIZE_SMALL: f32 = 28.0;  // link quality / other details

pub fn render(frame: &mut GlesFrame, call: &CallSurface, now: u64) {
    // Background
    draw_rect(frame, 0, 0, SCREEN_W, SCREEN_H, colors::BACKGROUND);

    // Status bar area
    draw_rect(frame, 0, 0, SCREEN_W, STATUS_H, colors::STATUS_BAR);

    // ── Contact avatar (upper third, centered) ──
    let avatar_cx = SCREEN_W / 2;
    let avatar_cy = STATUS_H + HEADER_MARGIN + AVATAR_R;
    draw_circle(frame, avatar_cx, avatar_cy, AVATAR_R, colors::ACCENT);

    // ── Contact name (below avatar) ──
    let name = call.display_name();
    let name_y = avatar_cy + AVATAR_R + 40;
    let name_width = text_renderer::measure_text(&name, FONT_SIZE_LARGE);
    let name_x = (SCREEN_W - name_width) / 2;
    text_renderer::render_text(
        frame,
        &name,
        name_x,
        name_y,
        FONT_SIZE_LARGE,
        colors::TEXT_PRIMARY,
    );

    // ── Key fingerprint (under name) ──
    let key_display = format!("key: {}", call.key_fp);
    let key_y = name_y + 60;
    let key_width = text_renderer::measure_text(&key_display, FONT_SIZE_SMALL);
    let key_x = (SCREEN_W - key_width) / 2;
    text_renderer::render_text(
        frame,
        &key_display,
        key_x,
        key_y,
        FONT_SIZE_SMALL,
        colors::TEXT_SECONDARY,
    );

    // ── Call status (duration, ringing, connecting, etc.) ──
    let status = call.state.status_line(now);
    let status_y = key_y + 60;
    let status_width = text_renderer::measure_text(&status, FONT_SIZE_MEDIUM);
    let status_x = (SCREEN_W - status_width) / 2;
    text_renderer::render_text(
        frame,
        &status,
        status_x,
        status_y,
        FONT_SIZE_MEDIUM,
        colors::ACCENT_GREEN,
    );

    // ── Encryption pill ──
    let enc_text = "Ed25519 + ChaCha20";
    let enc_pill_y = status_y + 80;
    let enc_width = text_renderer::measure_text(enc_text, FONT_SIZE_SMALL);
    let enc_pill_x = (SCREEN_W - enc_width - 40) / 2; // +40 for padding
    let enc_pill_h = 50;
    let enc_pill_r = 25;

    draw_rounded_rect(
        frame,
        enc_pill_x - 20,
        enc_pill_y,
        enc_width + 40,
        enc_pill_h,
        enc_pill_r,
        colors::CARD,
    );

    text_renderer::render_text(
        frame,
        enc_text,
        enc_pill_x,
        enc_pill_y + 8,
        FONT_SIZE_SMALL,
        colors::ACCENT,
    );

    // ── Link quality indicator ──
    let link_quality = call.link_quality();
    let link_text = format!("Link: {}", link_quality.label());
    let link_y = enc_pill_y + enc_pill_h + 40;
    let link_width = text_renderer::measure_text(&link_text, FONT_SIZE_SMALL);
    let link_x = (SCREEN_W - link_width) / 2;
    text_renderer::render_text(
        frame,
        &link_text,
        link_x,
        link_y,
        FONT_SIZE_SMALL,
        colors::TEXT_SECONDARY,
    );

    // Signal bars (0-4)
    let bars = link_quality.bars();
    let bar_start_x = (SCREEN_W / 2) - 60;
    for i in 0..4 {
        let bar_x = bar_start_x + i as i32 * 30;
        let bar_h = 20 + (i as i32) * 8;
        let bar_color = if i < bars {
            colors::ACCENT_GREEN
        } else {
            colors::DIVIDER
        };
        draw_rect(frame, bar_x, link_y + 50, 20, bar_h, bar_color);
    }

    // ── Control row (lower third) ──
    let control_row_y = SCREEN_H - CONTROL_ROW_H;
    let third = SCREEN_W / 3;

    // Left: Mute button
    let mute_cx = third / 2;
    let mute_cy = control_row_y + CONTROL_ROW_H / 2;
    let mute_color = if call.muted {
        colors::ACCENT_RED
    } else {
        colors::CARD
    };
    draw_circle(frame, mute_cx, mute_cy, BUTTON_R, mute_color);
    let mute_text = if call.muted { "MUTED" } else { "MUTE" };
    let mute_w = text_renderer::measure_text(mute_text, FONT_SIZE_SMALL);
    text_renderer::render_text(
        frame,
        mute_text,
        mute_cx - mute_w / 2,
        mute_cy - 14,
        FONT_SIZE_SMALL,
        colors::TEXT_PRIMARY,
    );

    // Center: End call button
    let end_cx = SCREEN_W / 2;
    let end_cy = control_row_y + CONTROL_ROW_H / 2;
    draw_circle(frame, end_cx, end_cy, BUTTON_R, colors::HANGUP_RED);
    let end_text = "END";
    let end_w = text_renderer::measure_text(end_text, FONT_SIZE_SMALL);
    text_renderer::render_text(
        frame,
        end_text,
        end_cx - end_w / 2,
        end_cy - 14,
        FONT_SIZE_SMALL,
        colors::TEXT_PRIMARY,
    );

    // Right: Speaker button
    let spkr_cx = (third * 3) / 2;
    let spkr_cy = control_row_y + CONTROL_ROW_H / 2;
    let spkr_color = if call.speaker_on {
        colors::ACCENT_GREEN
    } else {
        colors::CARD
    };
    draw_circle(frame, spkr_cx, spkr_cy, BUTTON_R, spkr_color);
    let spkr_text = if call.speaker_on { "SPKR ON" } else { "SPEAKER" };
    let spkr_w = text_renderer::measure_text(spkr_text, FONT_SIZE_SMALL);
    text_renderer::render_text(
        frame,
        spkr_text,
        spkr_cx - spkr_w / 2,
        spkr_cy - 14,
        FONT_SIZE_SMALL,
        colors::TEXT_PRIMARY,
    );

    println!(
        "[call_ui_render] {} {} status:{} enc:yes link:{}",
        call.display_name(),
        call.key_fp,
        call.state.label(),
        call.link_quality().label(),
    );
}
