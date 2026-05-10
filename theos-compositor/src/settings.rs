// settings.rs — theOS Settings Surface
// Native settings panel rendered by the compositor
// Reads and writes system config via daemon IPC

use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

#[derive(Debug, Clone)]
pub struct Toggle {
    pub label: &'static str,
    pub sublabel: &'static str,
    pub enabled: bool,
}

pub struct Settings {
    pub toggles: Vec<Toggle>,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            toggles: vec![
                Toggle { label: "Starlink",            sublabel: "Satellite · Primary link",     enabled: true  },
                Toggle { label: "WiFi",                sublabel: "802.11ac · Backup link",       enabled: true  },
                Toggle { label: "LTE",                 sublabel: "4G · Fallback",                enabled: true  },
                Toggle { label: "AI Assistant",        sublabel: "Satellite-aware voice AI",     enabled: true  },
                Toggle { label: "Face Unlock",         sublabel: "On-device · Privacy first",    enabled: true  },
                Toggle { label: "Emotion Detection",   sublabel: "Voice stress analysis",        enabled: false },
                Toggle { label: "Smart Link Prediction",sublabel: "ML signal forecasting",       enabled: true  },
                Toggle { label: "Emergency Mode",      sublabel: "Auto-call on accident detect", enabled: true  },
            ],
        }
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(t) = self.toggles.get_mut(index) {
            t.enabled = !t.enabled;
            println!("[settings] {} → {}", t.label, t.enabled);
        }
    }

    /// Returns which toggle row was tapped
    pub fn row_at(&self, y: f64) -> Option<usize> {
        let list_top = 2400.0 * 0.15;
        let row_h = 2400.0 * 0.08;
        if y < list_top { return None; }
        let idx = ((y - list_top) / row_h) as usize;
        if idx < self.toggles.len() { Some(idx) } else { None }
    }
}

impl Surface for Settings {
    fn name(&self) -> &str { "settings" }

    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!("[settings] render — {} toggles", self.toggles.len());
        for t in &self.toggles {
            println!("  {} [{}]", t.label, if t.enabled { "ON" } else { "OFF" });
        }
    }

    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }
        // Toggle zone — right side of screen (toggle switch area)
        if x > 1080.0 * 0.75 {
            if let Some(idx) = self.row_at(y) {
                self.toggle(idx);
            }
        }
    }
}
