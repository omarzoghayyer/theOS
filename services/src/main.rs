// main.rs -- theOS Services
// Runs either the connectivity daemon or the DHT bootstrap server
// depending on the THEOS_MODE environment variable.
//
// THEOS_MODE=bootstrap  -> DHT bootstrap server (deploy to Railway.app)
// THEOS_MODE=daemon     -> Connectivity daemon (runs on device)
// (default)             -> daemon

mod voip;
mod audio;
mod bootstrap;
mod ntp_task;

#[tokio::main]
async fn main() {
    let mode = std::env::var("THEOS_MODE")
        .unwrap_or_else(|_| "daemon".to_string());

    match mode.as_str() {
        "bootstrap" => {
            bootstrap::run().await;
        }
        _ => {
            run_daemon().await;
        }
    }
}

async fn run_daemon() {
    println!("[daemon] theOS Connectivity Daemon starting");

    // Start NTP clock sync -- reliable time hardens replay protection.
    let clock = ntp_task::new_clock();
    ntp_task::spawn(clock.clone());
    println!("[daemon] NTP clock sync task started");

    let my_key = [0xAAu8; 32];

    let voip = voip::VoipEngine::new(my_key);
    println!("[daemon] VoIP engine ready -- state:{}", voip.state_label());

    match audio::AudioEngine::new() {
        Ok(_audio) => println!("[daemon] audio engine ready"),
        Err(e)     => println!("[daemon] audio engine unavailable: {} (ok in dev mode)", e),
    }

    println!("[daemon] ready -- waiting for IPC commands from compositor");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
