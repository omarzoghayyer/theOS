// voip.rs — VoIP/SIP Engine
// Makes and receives calls over satellite IP connection

use std::error::Error;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::UdpSocket;
use crate::network::NetworkLink;
use crate::config::SipConfig;

const RTP_PORT: u16 = 16384;

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
    config: SipConfig,
    link: NetworkLink,
    call_state: Mutex<CallState>,
}

impl VoipEngine {
    pub async fn new(link: NetworkLink, config: SipConfig) -> Result<Self, Box<dyn Error>> {
        let sip_address = format!("sip:{}@{}", config.username, config.server);
        println!("VoIP engine initialized on {} ({})", link.interface, link.ip_address);
        Ok(Self {
            inner: Arc::new(VoipInner {
                sip_address,
                config,
                link,
                call_state: Mutex::new(CallState::Idle),
            }),
        })
    }

    pub fn sip_address(&self) -> &str {
        &self.inner.sip_address
    }

    pub async fn make_call(&self, destination: &str) -> Result<(), Box<dyn Error>> {
        let mut state = self.inner.call_state.lock().await;
        if *state != CallState::Idle {
            return Err("Already in a call".into());
        }
        *state = CallState::Dialing;
        drop(state);

        println!("Initiating SIP INVITE to {}...", destination);
        let invite = self.build_sip_invite(destination);
        self.send_sip_message(&invite).await?;

        let response = self.wait_for_sip_response().await?;
        if response.contains("200 OK") {
            println!("Call connected!");
            self.send_sip_ack(destination).await?;
            *self.inner.call_state.lock().await = CallState::Active;
        } else {
            println!("Call failed: {}", response);
            *self.inner.call_state.lock().await = CallState::Idle;
        }

        Ok(())
    }

    pub async fn listen_for_calls(&self) {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.inner.config.port))
            .await
            .expect("Failed to bind SIP port");
        println!("Listening for incoming calls on port {}", self.inner.config.port);

        let mut buf = [0u8; 4096];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                    if msg.contains("INVITE") {
                        println!("Incoming call from {}!", addr);
                        let ok = self.build_sip_200_ok(&msg);
                        let _ = socket.send_to(ok.as_bytes(), addr).await;
                        *self.inner.call_state.lock().await = CallState::Active;
                        println!("Call active with {}", addr);
                    }
                }
                Err(e) => eprintln!("SIP listen error: {}", e),
            }
        }
    }

    pub async fn hangup(&self) -> Result<(), Box<dyn Error>> {
        let mut state = self.inner.call_state.lock().await;
        if *state == CallState::Active {
            println!("Sending SIP BYE...");
            let bye = self.build_sip_bye();
            self.send_sip_message(&bye).await?;
            *state = CallState::Idle;
            println!("Call ended.");
        }
        Ok(())
    }

    // ---- SIP message builders ----

    fn build_sip_invite(&self, destination: &str) -> String {
        format!(
            "INVITE sip:{dest}@{server} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {ip}:{port};branch=z9hG4bK-theos-001\r\n\
             From: <{from}>;tag=theos-01\r\n\
             To: <sip:{dest}@{server}>\r\n\
             Call-ID: theos-call-001@{ip}\r\n\
             CSeq: 1 INVITE\r\n\
             Authorization: Digest username=\"{user}\", realm=\"{server}\", uri=\"sip:{server}\", response=\"\"\r\n\
             Content-Length: 0\r\n\r\n",
            dest = destination,
            server = self.inner.config.server,
            ip = self.inner.link.ip_address,
            port = self.inner.config.port,
            from = self.inner.sip_address,
            user = self.inner.config.username,
        )
    }

    fn build_sip_200_ok(&self, _invite: &str) -> String {
        format!(
            "SIP/2.0 200 OK\r\n\
             From: <{from}>\r\n\
             To: <{from}>;tag=theos-02\r\n\
             Content-Length: 0\r\n\r\n",
            from = self.inner.sip_address,
        )
    }

    fn build_sip_bye(&self) -> String {
        format!(
            "BYE sip:remote@{server} SIP/2.0\r\n\
             From: <{from}>\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            server = self.inner.config.server,
            from = self.inner.sip_address,
        )
    }

    async fn send_sip_ack(&self, destination: &str) -> Result<(), Box<dyn Error>> {
        let ack = format!(
            "ACK sip:{dest}@{server} SIP/2.0\r\n\
             From: <{from}>\r\n\
             CSeq: 1 ACK\r\n\
             Content-Length: 0\r\n\r\n",
            dest = destination,
            server = self.inner.config.server,
            from = self.inner.sip_address,
        );
        self.send_sip_message(&ack).await
    }

    async fn send_sip_message(&self, message: &str) -> Result<(), Box<dyn Error>> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let target = format!("{}:{}", self.inner.config.server, self.inner.config.port);
        socket.send_to(message.as_bytes(), &target).await?;
        Ok(())
    }

    async fn wait_for_sip_response(&self) -> Result<String, Box<dyn Error>> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.inner.config.port)).await?;
        let mut buf = [0u8; 4096];
        let (len, _) = socket.recv_from(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf[..len]).to_string())
    }
}