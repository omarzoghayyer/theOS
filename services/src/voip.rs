// voip.rs — VoIP/SIP Engine
// Makes and receives calls over satellite IP connection
// Uses SIP for signaling, RTP for audio streaming

use std::error::Error;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::UdpSocket;

use crate::network::NetworkLink;
use crate::audio::AudioEngine;

const SIP_PORT: u16 = 5060;
const RTP_PORT: u16 = 16384;
const SIP_SERVER: &str = "sip.theos.io"; // Your SIP server — replace with actual

#[derive(Debug, Clone, PartialEq)]
pub enum CallState {
    Idle,
    Dialing,
    Ringing,
    Active,
    Ending,
}

#[derive(Clone)]
pub struct VoipEngine {
    inner: Arc<VoipInner>,
}

struct VoipInner {
    sip_address: String,
    link: NetworkLink,
    call_state: Mutex<CallState>,
    audio: AudioEngine,
}

impl VoipEngine {
    pub async fn new(link: NetworkLink) -> Result<Self, Box<dyn Error>> {
        let sip_address = format!("sip:device@{}", link.ip_address);
        let audio = AudioEngine::new()?;

        println!("VoIP engine initialized on {} ({})", link.interface, link.ip_address);

        Ok(Self {
            inner: Arc::new(VoipInner {
                sip_address,
                link,
                call_state: Mutex::new(CallState::Idle),
                audio,
            }),
        })
    }

    pub fn sip_address(&self) -> &str {
        &self.inner.sip_address
    }

    /// Make an outbound call to a SIP address or phone number
    pub async fn make_call(&self, destination: &str) -> Result<(), Box<dyn Error>> {
        let mut state = self.inner.call_state.lock().await;
        if *state != CallState::Idle {
            return Err("Already in a call".into());
        }
        *state = CallState::Dialing;
        drop(state);

        println!("Initiating SIP INVITE to {}...", destination);

        // Build SIP INVITE message
        let invite = self.build_sip_invite(destination);
        self.send_sip_message(&invite).await?;

        // Wait for 200 OK (simplified — production uses full SIP state machine)
        let response = self.wait_for_sip_response().await?;
        if response.contains("200 OK") {
            println!("Call connected!");
            self.send_sip_ack(destination).await?;
            *self.inner.call_state.lock().await = CallState::Active;
            self.start_rtp_stream().await?;
        } else {
            println!("Call failed: {}", response);
            *self.inner.call_state.lock().await = CallState::Idle;
        }

        Ok(())
    }

    /// Listen for incoming SIP calls
    pub async fn listen_for_calls(&self) {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", SIP_PORT))
            .await
            .expect("Failed to bind SIP port");

        println!("Listening for incoming calls on port {}", SIP_PORT);

        let mut buf = [0u8; 4096];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                    if msg.contains("INVITE") {
                        println!("Incoming call from {}!", addr);
                        self.handle_incoming_call(&msg, &socket, addr).await;
                    }
                }
                Err(e) => eprintln!("SIP listen error: {}", e),
            }
        }
    }

    async fn handle_incoming_call(
        &self,
        invite: &str,
        socket: &UdpSocket,
        from: std::net::SocketAddr,
    ) {
        println!("Ring ring! Accept call? (auto-accepting in Phase 1)");

        // Send 200 OK
        let ok = self.build_sip_200_ok(invite);
        let _ = socket.send_to(ok.as_bytes(), from).await;

        *self.inner.call_state.lock().await = CallState::Active;
        println!("Call active with {}", from);

        // Start RTP audio stream
        let _ = self.start_rtp_stream().await;
    }

    /// Start RTP audio — bidirectional voice stream
    async fn start_rtp_stream(&self) -> Result<(), Box<dyn Error>> {
        println!("Starting RTP audio stream on port {}", RTP_PORT);
        self.inner.audio.start_capture_and_playback(RTP_PORT).await?;
        Ok(())
    }

    /// End current call
    pub async fn hangup(&self) -> Result<(), Box<dyn Error>> {
        let state = self.inner.call_state.lock().await;
        if *state == CallState::Active {
            drop(state);
            println!("Sending SIP BYE...");
            let bye = self.build_sip_bye();
            self.send_sip_message(&bye).await?;
            *self.inner.call_state.lock().await = CallState::Idle;
            self.inner.audio.stop().await;
            println!("Call ended.");
        }
        Ok(())
    }

    // ---- SIP message builders (simplified — production uses full RFC 3261) ----

    fn build_sip_invite(&self, destination: &str) -> String {
        format!(
            "INVITE sip:{dest}@{server} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {ip}:{port};branch=z9hG4bK-theos-001\r\n\
             From: <{from}>;tag=theos-01\r\n\
             To: <sip:{dest}@{server}>\r\n\
             Call-ID: theos-call-001@{ip}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: 0\r\n\r\n",
            dest = destination,
            server = SIP_SERVER,
            ip = self.inner.link.ip_address,
            port = SIP_PORT,
            from = self.inner.sip_address,
        )
    }

    fn build_sip_200_ok(&self, _invite: &str) -> String {
        format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP {ip}:{port}\r\n\
             From: <{from}>\r\n\
             To: <{from}>;tag=theos-02\r\n\
             Content-Length: 0\r\n\r\n",
            ip = self.inner.link.ip_address,
            port = SIP_PORT,
            from = self.inner.sip_address,
        )
    }

    fn build_sip_bye(&self) -> String {
        format!(
            "BYE sip:remote@{server} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {ip}:{port}\r\n\
             From: <{from}>\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            server = SIP_SERVER,
            ip = self.inner.link.ip_address,
            port = SIP_PORT,
            from = self.inner.sip_address,
        )
    }

    async fn send_sip_ack(&self, destination: &str) -> Result<(), Box<dyn Error>> {
        let ack = format!(
            "ACK sip:{dest}@{server} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {ip}:{port}\r\n\
             CSeq: 1 ACK\r\n\
             Content-Length: 0\r\n\r\n",
            dest = destination,
            server = SIP_SERVER,
            ip = self.inner.link.ip_address,
            port = SIP_PORT,
        );
        self.send_sip_message(&ack).await
    }

    async fn send_sip_message(&self, message: &str) -> Result<(), Box<dyn Error>> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let target = format!("{}:{}", SIP_SERVER, SIP_PORT);
        socket.send_to(message.as_bytes(), &target).await?;
        Ok(())
    }

    async fn wait_for_sip_response(&self) -> Result<String, Box<dyn Error>> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", SIP_PORT)).await?;
        let mut buf = [0u8; 4096];
        let (len, _) = socket.recv_from(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf[..len]).to_string())
    }
}
