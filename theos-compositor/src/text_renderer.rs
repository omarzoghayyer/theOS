// text_renderer.rs — fontdue-based text rendering for theOS compositor

use fontdue::{Font, FontSettings};
use std::sync::OnceLock;

static FONT: OnceLock<Option<Font>> = OnceLock::new();

pub fn init_font() {
    let font = try_load_font();
    let _ = FONT.set(font);
}

fn try_load_font() -> Option<Font> {
    // Try system DejaVuSansMono first
    if let Ok(data) = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf") {
        if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
            return Some(font);
        }
    }
    
    // Try alternative paths
    let paths = [
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/System/Library/Fonts/Monaco.dfont",
    ];
    
    for path in &paths {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
                eprintln!("[text] Loaded font from {}", path);
                return Some(font);
            }
        }
    }
    
    eprintln!("[text] No system fonts found, text rendering will be disabled");
    None
}

pub fn font() -> Option<&'static Font> {
    FONT.get_or_init(try_load_font).as_ref()
}

pub fn render_text(
    frame: &mut smithay::backend::renderer::gles::GlesFrame,
    text: &str,
    x: i32,
    y: i32,
    font_size: f32,
    color: [f32; 4],
) -> i32 {
    if let Some(font) = font() {
        let mut current_x = x;

        for ch in text.chars() {
            let (metrics, bitmap) = font.rasterize(ch, font_size);

            if !bitmap.is_empty() {
                for (i, &alpha) in bitmap.iter().enumerate() {
                    if alpha > 0 {
                        let glyph_x = (i % metrics.width) as i32;
                        let glyph_y = (i / metrics.width) as i32;

                        let px = current_x + glyph_x;
                        let py = y + glyph_y;

                        if alpha > 128 {
                            crate::render::draw_rect(frame, px, py, 1, 1, color);
                        }
                    }
                }
            }

            current_x += metrics.advance_width as i32;
        }

        current_x - x
    } else {
        0
    }
}

pub fn measure_text(text: &str, font_size: f32) -> i32 {
    if let Some(font) = font() {
        let mut width = 0;
        for ch in text.chars() {
            let (metrics, _) = font.rasterize(ch, font_size);
            width += metrics.advance_width as i32;
        }
        width
    } else {
        (text.len() as i32) * (font_size as i32 / 2)
    }
}
