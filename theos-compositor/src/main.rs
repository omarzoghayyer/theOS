// main.rs -- theOS Wayland Compositor
#[cfg(feature = "compositor")] mod render;
#[cfg(feature = "compositor")] mod ipc;
#[cfg(feature = "compositor")] mod shell;
#[cfg(feature = "compositor")] mod dialer;
#[cfg(feature = "compositor")] mod assistant;
#[cfg(feature = "compositor")] mod settings;
#[cfg(feature = "compositor")] mod keystore;
#[cfg(feature = "compositor")] mod input;
#[cfg(feature = "compositor")] mod ai_shell;
mod crypto;
mod dht;
mod identity;
mod hal;

#[cfg(feature = "compositor")]
use render::{RenderPipeline, ActiveSurface, Surface, TouchState};
#[cfg(feature = "compositor")] use shell::Shell;
#[cfg(feature = "compositor")] use dialer::Dialer;
#[cfg(feature = "compositor")] use assistant::Assistant;
#[cfg(feature = "compositor")] use ai_shell::{AiShell, AiShellNav, AiShellState, InputMode};

fn main() {
    println!("theOS Wayland Compositor");
    println!("Satellite-First Mobile OS");

    #[cfg(not(feature = "compositor"))]
    {
        println!("[compositor] built without compositor feature -- run with --features compositor for full build");
        return;
    }

    #[cfg(feature = "compositor")]
    {
        let mut pipeline  = RenderPipeline::new(1080, 2280);
        let mut shell     = Shell::new();
        let dialer        = Dialer::new();
        let assistant     = Assistant::new();
        let mut ai_shell  = AiShell::new();

        println!("[compositor] ai_shell:  {}", ai_shell.name());
        println!("[compositor] shell:     {}", shell.name());
        println!("[compositor] dialer:    {}", dialer.name());
        println!("[compositor] assistant: {}", assistant.name());

        pipeline.navigate(ActiveSurface::Lock);
        shell.unlock();
        pipeline.navigate(ActiveSurface::AiShell);

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

        ai_shell.input_buf  = "show connection".to_string();
        ai_shell.input_mode = InputMode::Typing;
        let nav = ai_shell.submit_input();
        println!("[compositor] nav after 'show connection': {:?}", nav);

        ai_shell.input_buf  = "call sarah".to_string();
        ai_shell.input_mode = InputMode::Typing;
        let nav = ai_shell.submit_input();

        match nav {
            AiShellNav::GoToDialer { contact } => {
                println!("[compositor] AI -> dialer for '{}'", contact);
                pipeline.navigate(ActiveSurface::Dialer);
            }
            AiShellNav::GoToTraditionalHome => {
                pipeline.navigate(ActiveSurface::Home);
            }
            _ => {}
        }

        println!("[compositor] ready -- waiting for OnePlus 6 hardware");
    }
}
