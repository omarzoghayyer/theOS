mod network;
mod voip;
mod audio;
mod config;

#[tokio::main]
async fn main() {
    println!("theOS Connectivity Daemon starting...");

    // Load config
    let sip_config = config::SipConfig::load().expect("Failed to load config.toml");
    println!("Loaded SIP config for user: {}", sip_config.username);

    // Network
    let net_manager = network::NetworkManager::new().await.unwrap();
    let link = net_manager.best_link().await;
    println!("Active link: {}", link);

    // Audio
    let audio = audio::AudioEngine::new().unwrap();
    audio.start().await;

    // VoIP
    let voip = voip::VoipEngine::new(link, sip_config).await.unwrap();
    println!("SIP address: {}", voip.sip_address());

    // Make a real test call to device2
    voip.register().await.unwrap();
    voip.make_call("theos_device2@sip.linphone.org").await.unwrap();}