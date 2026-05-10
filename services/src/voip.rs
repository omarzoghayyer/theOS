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

    pub async fn register(&self) -> Result<(), Box<dyn Error>> {
    use std::net::ToSocketAddrs;
    println!("Registering with SIP server {}...", self.inner.config.server);

    let target = format!("{}:{}", self.inner.config.server, self.inner.config.port);
    let addr = target.to_socket_addrs()?
        .find(|a| a.is_ipv4())
        .ok_or("No IPv4 address found")?;

    // Bind to a fixed port so server can send response back to us
    let socket = UdpSocket::bind("0.0.0.0:5060").await
        .unwrap_or(UdpSocket::bind("0.0.0.0:0").await?);
    
    let uri = format!("sip:{}", self.inner.config.server);

        let register = self.build_register_msg(&uri, None, 1);      
        socket.send_to(register.as_bytes(), addr).await?;

        let mut buf = [0u8; 4096];
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                if let Ok((len, _)) = result {
                    let response = String::from_utf8_lossy(&buf[..len]).to_string();
                    if response.contains("401") {
                        let realm = extract_param(&response, "realm").unwrap_or_default();
                        let nonce = extract_param(&response, "nonce").unwrap_or_default();
                        let opaque = extract_param(&response, "opaque").unwrap_or_default();
                        println!("Got 401, realm={}", realm);

                        let auth = self.build_digest_auth(&realm, &nonce, &opaque, "REGISTER", &uri);
                        let register_auth = self.build_register_msg(&uri, Some(&auth), 2);
                        socket.send_to(register_auth.as_bytes(), addr).await?;

                        let mut buf2 = [0u8; 4096];
                        tokio::select! {
                            result2 = socket.recv_from(&mut buf2) => {
                                if let Ok((len2, _)) = result2 {
                                    let resp2 = String::from_utf8_lossy(&buf2[..len2]);
                                    if resp2.contains("200") {
                                        println!("Registered successfully!");
                                    } else {
                                        println!("Registration failed: {}", &resp2[..resp2.len().min(100)]);
                                    }
                                }
                            }
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {
                                println!("Registration timeout.");
                            }
                        }
                    } else if response.contains("200 OK") {
                        println!("Registered successfully!");
                    }
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {
                println!("Registration timeout.");
            }
        }
        Ok(())
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

   fn build_register_msg(&self, uri: &str, auth: Option<&str>, cseq: u32) -> String {
    let auth_header = match auth {
        Some(a) => format!("Authorization: {}\r\n", a),
        None => String::new(),
    };
    let call_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!(
        "REGISTER {uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {ip}:5060;branch=z9hG4bK-{call_id}-{cseq}\r\n\
         From: <sip:{user}@{server}>;tag=theos-{call_id}\r\n\
         To: <sip:{user}@{server}>\r\n\
         Call-ID: {call_id}@{ip}\r\n\
         CSeq: {cseq} REGISTER\r\n\
         Contact: <sip:{user}@{ip}:5060>\r\n\
         Expires: 3600\r\n\
         {auth}Content-Length: 0\r\n\r\n",
        uri = uri,
        ip = self.inner.link.ip_address,
        server = self.inner.config.server,
        user = self.inner.config.username,
        auth = auth_header,
        cseq = cseq,
        call_id = call_id,
    )
}

       fn build_digest_auth(&self, realm: &str, nonce: &str, opaque: &str, method: &str, uri: &str) -> String {
        let nc = "00000001";
        let cnonce = "theos-cnonce-01";
    
        let ha1 = format!("{:x}", md5::compute(format!(
            "{}:{}:{}", self.inner.config.username, realm, self.inner.config.password
        )));
        let ha2 = format!("{:x}", md5::compute(format!("{}:{}", method, uri)));
        let response = format!("{:x}", md5::compute(format!(
            "{}:{}:{}:{}:auth:{}", ha1, nonce, nc, cnonce, ha2
        )));

        format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", \
             nc={}, cnonce=\"{}\", qop=auth, response=\"{}\", opaque=\"{}\", algorithm=MD5",
            self.inner.config.username, realm, nonce, uri, nc, cnonce, response, opaque
        )
    }

    fn build_sip_invite(&self, destination: &str) -> String {
    let call_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!(
        "INVITE sip:{dest}@{server} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {ip}:5060;branch=z9hG4bK-invite-{call_id}\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:{user}@{server}>;tag=theos-{call_id}\r\n\
         To: <sip:{dest}@{server}>\r\n\
         Call-ID: call-{call_id}@{ip}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:{user}@{ip}:5060>\r\n\
         Authorization: {auth}\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: 0\r\n\r\n",
        dest = destination,
        server = self.inner.config.server,
        ip = self.inner.link.ip_address,
        user = self.inner.config.username,
        call_id = call_id,
        auth = self.build_digest_auth(
            &self.inner.config.server,
            "placeholder",
            "",
            "INVITE",
            &format!("sip:{}@{}", destination, self.inner.config.server)
        ),
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
        use std::net::ToSocketAddrs;
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let target = format!("{}:{}", self.inner.config.server, self.inner.config.port);
        let addr = target.to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or("No IPv4 address found for SIP server")?;
        socket.send_to(message.as_bytes(), addr).await?;
        Ok(())
    }

    async fn wait_for_sip_response(&self) -> Result<String, Box<dyn Error>> {
    let socket = UdpSocket::bind("0.0.0.0:5060").await
        .unwrap_or(UdpSocket::bind("0.0.0.0:0").await?);
    let mut buf = [0u8; 4096];
    tokio::select! {
        result = socket.recv_from(&mut buf) => {
            let (len, _) = result?;
            Ok(String::from_utf8_lossy(&buf[..len]).to_string())
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
            Ok("Timeout — no response".to_string())
        }
    }
}
}

fn extract_param(response: &str, param: &str) -> Option<String> {
    let key = format!("{}=\"", param);
    let start = response.find(&key)? + key.len();
    let end = response[start..].find('"')? + start;
    Some(response[start..end].to_string())
}