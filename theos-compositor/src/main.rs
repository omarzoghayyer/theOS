// main.rs — theOS Wayland Compositor
// Real display server — no web layer, no browser
// Runs directly on DRM/KMS hardware

mod render;
mod ipc;
mod shell;
mod dialer;
mod assistant;
mod settings;
mod keystore;
mod input;

use render::{RenderPipeline, ActiveSurface, Surface, TouchState};
use shell::Shell;
use dialer::Dialer;
use assistant::Assistant;

fn main() {
    println!("╔════════════════════════════════════╗");
    println!("║     theOS Wayland Compositor        ║");
    println!("║     Satellite-First Mobile OS       ║");
    println!("╚════════════════════════════════════╝");

    // Pixel 6 display: 1080x2400
    let mut pipeline  = RenderPipeline::new(1080, 2400);
    let mut shell     = Shell::new();
    let mut dialer    = Dialer::new();
    let mut assistant = Assistant::new();

    println!("[compositor] surfaces initialized");
    println!("[compositor] shell:     {}", shell.name());
    println!("[compositor] dialer:    {}", dialer.name());
    println!("[compositor] assistant: {}", assistant.name());

    // Boot → lock screen
    println!("[compositor] boot → lock screen");
    pipeline.navigate(ActiveSurface::Lock);

    // Unlock
    shell.handle_touch(540.0, 1200.0, TouchState::Down);
    pipeline.navigate(ActiveSurface::Home);

    // Tap Phone app
    shell.handle_touch(135.0, 1200.0, TouchState::Down);
    pipeline.navigate(ActiveSurface::Dialer);

    // Dial 456
    dialer.handle_touch(180.0, 1200.0, TouchState::Down);
    dialer.handle_touch(540.0, 1200.0, TouchState::Down);
    dialer.handle_touch(900.0, 1200.0, TouchState::Down);
    println!("[compositor] dialer number: {}", dialer.display_number());

    println!("[compositor] ready — waiting for Pixel 6 hardware");
}
