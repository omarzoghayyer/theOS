// network.rs — Network Interface Manager
// Detects available links: Starlink, LTE, WiFi
// Picks best available link and monitors quality

use std::error::Error;
use std::fmt;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq)]
pub enum LinkType {
    Starlink,
    LTE,
    WiFi,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct NetworkLink {
    pub link_type: LinkType,
    pub interface: String,      // e.g. "eth0", "wlan0", "rmnet0"
    pub ip_address: String,
    pub latency_ms: u32,
    pub signal_quality: f32,    // 0.0 - 1.0
    pub is_active: bool,
}

impl fmt::Display for NetworkLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {} ip={} latency={}ms quality={:.0}%",
            self.link_type,
            self.interface,
            self.ip_address,
            self.latency_ms,
            self.signal_quality * 100.0
        )
    }
}

pub struct NetworkManager {
    links: Vec<NetworkLink>,
}

impl NetworkManager {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        println!("Scanning network interfaces...");
        let links = Self::scan_interfaces().await;
        Ok(Self { links })
    }

    /// Scan system network interfaces and classify them
    async fn scan_interfaces() -> Vec<NetworkLink> {
        let mut links = Vec::new();

        // In production this reads from /sys/class/net and checks routing tables
        // For now we detect interfaces by name patterns and probe them
        let interfaces = get_system_interfaces();

        for iface in interfaces {
            let link_type = classify_interface(&iface);
            let ip = get_interface_ip(&iface).unwrap_or_else(|| "0.0.0.0".to_string());
            let latency = probe_latency(&iface).await;
            let quality = estimate_quality(&link_type, latency);

            let link = NetworkLink {
                link_type,
                interface: iface,
                ip_address: ip,
                latency_ms: latency,
                signal_quality: quality,
                is_active: latency < 9999,
            };

            println!("  Found: {}", link);
            links.push(link);
        }

        links
    }

    /// Returns the best available link, prioritizing Starlink
    pub async fn best_link(&self) -> NetworkLink {
        // Priority: Starlink > WiFi > LTE > Unknown
        let priority = |l: &NetworkLink| match l.link_type {
            LinkType::Starlink => 0,
            LinkType::WiFi     => 1,
            LinkType::LTE      => 2,
            LinkType::Unknown  => 3,
        };

        self.links
            .iter()
            .filter(|l| l.is_active)
            .min_by_key(|l| priority(l))
            .cloned()
            .unwrap_or_else(|| NetworkLink {
                link_type: LinkType::Unknown,
                interface: "none".to_string(),
                ip_address: "0.0.0.0".to_string(),
                latency_ms: 9999,
                signal_quality: 0.0,
                is_active: false,
            })
    }

    /// Monitor links continuously and trigger handoff if quality drops
    pub async fn monitor_links<F>(&self, mut on_handoff: F)
    where
        F: FnMut(NetworkLink),
    {
        println!("Starting link quality monitor...");
        let mut current_best = self.best_link().await;

        loop {
            sleep(Duration::from_secs(5)).await;
            let new_links = Self::scan_interfaces().await;
            let manager = NetworkManager { links: new_links };
            let new_best = manager.best_link().await;

            // Trigger handoff if better link available or quality degraded
            if new_best.interface != current_best.interface
                || new_best.signal_quality > current_best.signal_quality + 0.2
            {
                println!("Handoff: {} -> {}", current_best, new_best);
                on_handoff(new_best.clone());
                current_best = new_best;
            }
        }
    }
}

// ---- Helper functions (will read from /sys/class/net in production) ----

fn get_system_interfaces() -> Vec<String> {
    // Production: read /sys/class/net directory
    // For now return common interface names to scan
    vec![
        "eth0".to_string(),   // Starlink dish typically appears as ethernet
        "wlan0".to_string(),  // WiFi
        "rmnet0".to_string(), // LTE modem
        "usb0".to_string(),   // USB tethering / Starlink USB
    ]
}

fn classify_interface(iface: &str) -> LinkType {
    // Starlink dish connects via ethernet or USB
    // In production we'd also check MAC OUI for Starlink hardware
    match iface {
        i if i.starts_with("eth") || i.starts_with("usb") => LinkType::Starlink,
        i if i.starts_with("wlan") => LinkType::WiFi,
        i if i.starts_with("rmnet") || i.starts_with("wwan") => LinkType::LTE,
        _ => LinkType::Unknown,
    }
}

fn get_interface_ip(iface: &str) -> Option<String> {
    // Read real IP from system using local socket trick
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return Some(addr.ip().to_string());
            }
        }
    }
    match iface {
        "eth0" => Some("192.168.100.2".to_string()),
        "wlan0" => Some("192.168.1.100".to_string()),
        _ => None,
    }
}

async fn probe_latency(iface: &str) -> u32 {
    // Production: send ICMP ping through specific interface
    // Stub: simulate realistic latencies
    match iface {
        "eth0"  => 45,   // Starlink ~20-60ms typical
        "wlan0" => 15,   // Local WiFi
        "rmnet0"=> 80,   // LTE
        _ => 9999,       // Unreachable
    }
}

fn estimate_quality(link_type: &LinkType, latency_ms: u32) -> f32 {
    if latency_ms >= 9999 { return 0.0; }
    // Simple quality score based on latency thresholds per link type
    let max_acceptable = match link_type {
        LinkType::Starlink => 150.0,
        LinkType::WiFi     => 50.0,
        LinkType::LTE      => 200.0,
        LinkType::Unknown  => 500.0,
    };
    let quality = 1.0 - (latency_ms as f32 / max_acceptable).min(1.0);
    quality
}
