// theOS — Connectivity Daemon
// Phase 1: Core satellite connectivity + VoIP calling
// Target: Pixel device over Starlink IP connection

mod network;
mod voip;
mod audio;

use std::error::Error;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("theOS Connectivity Daemon starting...");

    // 1. Initialize network manager — detect and connect to Starlink
    let net_manager = network::NetworkManager::new().await?;
    let link = net_manager.best_link().await;
    println!("Active link: {:?}", link);

    // 2. Initialize VoIP engine on top of active link
    let voip_engine = voip::VoipEngine::new(link).await?;
    println!("VoIP engine ready. SIP address: {}", voip_engine.sip_address());

    // 3. Start listening for incoming calls
    let voip_clone = voip_engine.clone();
    tokio::spawn(async move {
        voip_clone.listen_for_calls().await;
    });

    // 4. CLI for making calls (Phase 1 — no UI yet)
    cli_dialer(voip_engine).await;

    signal::ctrl_c().await?;
    println!("Shutting down theOS daemon.");
    Ok(())
}

async fn cli_dialer(engine: voip::VoipEngine) {
    use std::io::{self, BufRead, Write};
    let stdin = io::stdin();
    loop {
        print!("theOS> Enter number to call (or 'quit'): ");
        io::stdout().flush().unwrap();
        let mut line = String::new();
        stdin.lock().read_line(&mut line).unwrap();
        let input = line.trim();
        if input == "quit" { break; }
        if !input.is_empty() {
            println!("Dialing {}...", input);
            match engine.make_call(input).await {
                Ok(_) => println!("Call connected."),
                Err(e) => println!("Call failed: {}", e),
            }
        }
    }
}
