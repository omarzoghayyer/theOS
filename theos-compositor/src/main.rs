// main.rs -- theOS Wayland Compositor
// Real display server -- no web layer, no browser
// Runs directly on DRM/KMS hardware
mod render;
mod ipc;
mod shell;
mod dialer;
mod assistant;
mod settings;
mod keystore;
mod input;
mod ai_shell;
mod crypto;

use render::{RenderPipeline, ActiveSurface, Surface, TouchState};
use shell::Shell;
use dialer::Dialer;
use assistant::Assistant;
use ai_shell::{AiShell, AiShellNav, AiShellState, InputMode};

fn main() {
    println!("theOS Wayland Compositor");
    println!("Satellite-First Mobile OS");

    // OnePlus 6: 1080x2280
    let mut pipeline  = RenderPipeline::new(1080, 2280);
    let mut shell     = Shell::new();
    let mut dialer    = Dialer::new();
    let mut assistant = Assistant::new();
    let mut ai_shell  = AiShell::new();

    println!("[compositor] surfaces initialized");
    println!("[compositor] ai_shell:  {}", ai_shell.name());
    println!("[compositor] shell:     {}", shell.name());
    println!("[compositor] dialer:    {}", dialer.name());
    println!("[compositor] assistant: {}", assistant.name());

    // Boot -> lock screen
    println!("[compositor] boot -> lock screen");
    pipeline.navigate(ActiveSurface::Lock);

    // Unlock -> AI shell (not traditional home)
    shell.unlock();
    pipeline.navigate(ActiveSurface::AiShell);

    // Push live system state into the AI shell
    ai_shell.update_state(AiShellState {
        handle:         "omar".to_string(),
        pubkey_hex:     "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        satellite_link: shell.satellite_link.clone(),
        latency_ms:     shell.latency_ms,
        signal_quality: shell.signal_quality,
        dht_peers:      0,
        contact_count:  0,
        battery_pct:    shell.battery_pct,
        charging:       shell.charging,
    });

    // Simulate: user asks "show connection" -- answered inline, no navigation
    ai_shell.input_buf  = "show connection".to_string();
    ai_shell.input_mode = InputMode::Typing;
    let nav = ai_shell.submit_input();
    println!("[compositor] nav after 'show connection': {:?}", nav);

    // Simulate: user says "call sarah" -- navigates to dialer
    ai_shell.input_buf  = "call sarah".to_string();
    ai_shell.input_mode = InputMode::Typing;
    let nav = ai_shell.submit_input();

    match nav {
        AiShellNav::GoToDialer { contact } => {
            println!("[compositor] AI -> dialer for '{}'", contact);
            pipeline.navigate(ActiveSurface::Dialer);
        }
        AiShellNav::GoToMessages => {
            println!("[compositor] AI -> messages");
        }
        AiShellNav::GoToSettings => {
            println!("[compositor] AI -> settings");
            pipeline.navigate(ActiveSurface::Settings);
        }
        AiShellNav::GoToTraditionalHome => {
            println!("[compositor] AI -> traditional home (escape hatch)");
            pipeline.navigate(ActiveSurface::Home);
        }
        AiShellNav::None => {}
    }

    println!("[compositor] ai_shell messages: {}", ai_shell.messages.len());
    println!("[compositor] ready -- waiting for OnePlus 6 hardware");
}
