// render.rs — theOS GLES Render Pipeline
// Draws all surfaces directly to the Pixel 6 display via DRM/KMS

use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

pub trait Surface {
    fn draw(&self, frame: &mut GlesFrame, size: Size<i32, Physical>);
    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState);
    fn name(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchState { Down, Up, Move }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveSurface { Lock, Home, Dialer, Assistant, Settings }

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
        println!("[render] switching to {:?}", surface);
        self.active_surface = surface;
    }
}
