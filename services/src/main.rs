// main.rs -- theOS Connectivity Daemon
// No SIP. No phone numbers. No carriers.
// Voice calls via Ed25519 identity + ChaCha20 encryption over satellite UDP.

mod voip;
mod audio;

#[tokio::main]
async fn main() {
    println!("[daemon] theOS Connectivity Daemon starting");

    // Identity key -- in production loaded from ARM TrustZone / OP-TEE
    // Demo: fixed test key, replaced by real Ed25519 keypair on device
    let my_key = [0xAAu8; 32];

    // VoIP engine -- no SIP, no carrier, no phone number
    let voip = voip::VoipEngine::new(my_key);
    println!("[daemon] VoIP engine ready -- state:{}", voip.state_label());

    // Audio engine
    match audio::AudioEngine::new() {
        Ok(_audio) => println!("[daemon] audio engine ready"),
        Err(e)     => println!("[daemon] audio engine unavailable: {} (ok in dev mode)", e),
    }

    println!("[daemon] ready -- waiting for IPC commands from compositor");

    // Production: Unix domain socket IPC loop driven by compositor
    // Demo: park and wait
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
