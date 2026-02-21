// ipc.rs — IPC Client for compositor apps
use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde::{Serialize, Deserialize};

const SOCKET_PATH_DEV: &str = "/tmp/theos-daemon.sock";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IpcMessage {
    Call { destination: String },
    Hangup,
    GetStatus,
    NetworkStatus { link: String, latency_ms: u32, quality: u8 },
    BatteryStatus { percent: u8, charging: bool },
    SystemMetrics { cpu: f32, memory_mb: u32, temp_c: f32 },
    SipRegistered { server: String },
    Error { code: u32, message: String },
}

pub struct IpcClient { stream: UnixStream }

impl IpcClient {
    pub async fn connect(_dev_mode: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let stream = UnixStream::connect(SOCKET_PATH_DEV).await?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: IpcMessage) -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = serde_json::to_vec(&msg)?;
        bytes.push(b'\n');
        self.stream.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<IpcMessage, Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 4096];
        let n = self.stream.read(&mut buf).await?;
        Ok(serde_json::from_slice(&buf[..n])?)
    }

    pub async fn call(&mut self, destination: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send(IpcMessage::Call { destination: destination.to_string() }).await
    }

    pub async fn get_status(&mut self) -> Result<IpcMessage, Box<dyn std::error::Error>> {
        self.send(IpcMessage::GetStatus).await?;
        self.recv().await
    }
}
