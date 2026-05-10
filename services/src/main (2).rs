// main.rs — theOS Connectivity Daemon
// Satellite-first mobile OS daemon
// Boots the full OS: network, VoIP, UI server, system monitor

mod network;
mod voip;
mod audio;
mod config;
mod server;
mod sysmon;
mod launcher;

use std::sync::Arc;

#[tokio::main]
async fn main() {
    println!("╔════════════════════════════════════╗");
    println!("║         theOS v0.1.0-alpha          ║");
    println!("║    Satellite-First Mobile OS        ║");
    println!("╚════════════════════════════════════╝");
    println!();

    // ── Device info ──
    let launch = launcher::Launcher::new();
    let device = launch.device_info();
    println!("Device  : {}", device.model);
    println!("Kernel  : {}", device.kernel);
    println!("Hardware: {}", if device.is_pixel { "Pixel 6 ✓" } else { "Dev machine" });
    println!();

    // ── Load config ──
    let sip_config = config::SipConfig::load().expect("Failed to load config.toml");
    println!("SIP user: {}", sip_config.username);

    // ── Network ──
    println!("\n── Network ──");
    let net = network::NetworkManager::new().await.unwrap();
    let link = net.best_link().await;
    println!("Active link: {}", link);

    // ── Audio ──
    println!("\n── Audio ──");
    let audio = audio::AudioEngine::new().unwrap();
    audio.start().await;

    // ── VoIP ──
    println!("\n── VoIP ──");
    let voip = Arc::new(
        voip::VoipEngine::new(link.clone(), sip_config).await.unwrap()
    );
    println!("SIP: {}", voip.sip_address());

    // ── UI Server ──
    println!("\n── UI Server ──");
    let ui = server::UiServer::new();
    let event_tx = ui.event_tx.clone();

    // ── System Monitor ──
    let sysmon = Arc::new(sysmon::SystemMonitor::new());
    let mon_tx = event_tx.clone();
    tokio::spawn(async move {
        sysmon.start_monitoring(mon_tx).await;
    });

    // ── SIP Registration ──
    let voip_reg = voip.clone();
    let reg_tx = event_tx.clone();
    tokio::spawn(async move {
        match voip_reg.register().await {
            Ok(_) => {
                let _ = reg_tx.send(r#"{"event":"sip_registered","status":"ok"}"#.to_string());
            }
            Err(e) => eprintln!("SIP registration error: {}", e),
        }
    });

    // ── Launch UI (kiosk on real hardware) ──
    if device.is_pixel {
        println!("Launching kiosk UI...");
        let _ = launch.launch_kiosk();
    } else {
        println!("Dev mode — open http://localhost:8080 in your browser");
    }

    // ── Start UI server (blocks) ──
    println!("\n── theOS Running ──\n");
    ui.start(8080).await;
}
