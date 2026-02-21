// shell.rs — theOS Home Shell Surface
use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

#[derive(Debug, Clone, PartialEq)]
pub enum ShellState { Locked, Home, AppSwitcher }

pub struct Shell {
    pub state: ShellState,
    pub battery_pct: u8,
    pub charging: bool,
    pub satellite_link: String,
    pub latency_ms: u32,
    pub signal_quality: u8,
    pub time_str: String,
    pub date_str: String,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            state: ShellState::Locked,
            battery_pct: 100,
            charging: false,
            satellite_link: "Starlink".to_string(),
            latency_ms: 0,
            signal_quality: 0,
            time_str: "00:00".to_string(),
            date_str: String::new(),
        }
    }

    pub fn unlock(&mut self) { self.state = ShellState::Home; }
    pub fn lock(&mut self) { self.state = ShellState::Locked; }

    pub fn apps() -> &'static [&'static str] {
        &["Phone","AI","Messages","Settings","Satellite","Security","Coverage","Dev"]
    }

    pub fn app_at(&self, x: f64, y: f64) -> Option<&'static str> {
        let grid_top = 2400.0 * 0.45;
        let cell_w = (1080.0 * 0.88) / 4.0;
        let cell_h = cell_w;
        let grid_left = 1080.0 * 0.06;
        if y < grid_top { return None; }
        let col = ((x - grid_left) / cell_w) as usize;
        let row = ((y - grid_top) / cell_h) as usize;
        Self::apps().get(row * 4 + col).copied()
    }
}

impl Surface for Shell {
    fn name(&self) -> &str { "shell" }
    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!("[shell] render {:?} — {}% bat, {} {}ms",
            self.state, self.battery_pct, self.satellite_link, self.latency_ms);
    }
    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }
        match self.state {
            ShellState::Locked => self.unlock(),
            ShellState::Home => {
                if let Some(app) = self.app_at(x, y) {
                    println!("[shell] app tapped: {}", app);
                }
            }
            _ => {}
        }
    }
}
