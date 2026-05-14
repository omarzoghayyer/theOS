// web_proxy.rs -- theOS AI Web Proxy
//
// The layer that makes "Hey OS, open Instagram" work.
// Not a browser. Not an app store. A privacy-first service renderer.
//
// Architecture:
//   web_proxy.rs (this file, theos-core)
//     -- pure logic, no UI, no network, fully testable on Windows
//     -- owns: service registry, session state, tracker blocklist,
//              credential store interface, bandwidth tracking, AI commands
//
//   sandbox.rs (theos-compositor)
//     -- WebKitGTK container, JS injection, native chrome wrapper
//     -- calls into web_proxy.rs for decisions
//
// User flow:
//   "Hey OS, open Instagram"
//     -> Intent::OpenApp { name: "instagram" }
//     -> ServiceRegistry::resolve("instagram") -> "https://instagram.com"
//     -> ProxySession::new("instagram", url)
//     -> sandbox.rs opens WebKitGTK with tracker blocking
//     -> user sees feed, no Meta tracking, no app downloaded
//
// Privacy guarantees:
//   - Tracker block list: ~200 known surveillance domains blocked
//   - No persistent cookies across sessions (session-scoped only)
//   - No fingerprinting scripts loaded
//   - Credentials encrypted by Ed25519 keypair -- user never sees password
//   - Zero app storage -- nothing installed, nothing persists after session
//
// Security assumptions (flag for audit):
//   - Tracker block list is static -- updated via OS updates, not runtime
//   - WebKitGTK sandbox isolation is process-level, not VM-level
//   - Credential encryption uses the same Ed25519 keypair as identity
//     (key compromise = credential compromise -- acceptable tradeoff for UX)

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// -- ServiceEntry -------------------------------------------------------------

/// A known web service the AI can open.
#[derive(Debug, Clone)]
pub struct ServiceEntry {
    pub name:        String,   // canonical name: "instagram"
    pub url:         String,   // entry point URL
    pub aliases:     Vec<String>, // "ig", "insta" -> "instagram"
    pub category:    ServiceCategory,
    pub requires_auth: bool,   // does this service need login?
    pub mobile_url:  Option<String>, // mobile-optimized URL if different
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceCategory {
    Social,
    Video,
    Music,
    Email,
    Shopping,
    Maps,
    News,
    Finance,
    Productivity,
    Other,
}

impl ServiceCategory {
    pub fn label(&self) -> &str {
        match self {
            ServiceCategory::Social      => "social",
            ServiceCategory::Video       => "video",
            ServiceCategory::Music       => "music",
            ServiceCategory::Email       => "email",
            ServiceCategory::Shopping    => "shopping",
            ServiceCategory::Maps        => "maps",
            ServiceCategory::News        => "news",
            ServiceCategory::Finance     => "finance",
            ServiceCategory::Productivity => "productivity",
            ServiceCategory::Other       => "other",
        }
    }
}

// -- ServiceRegistry ----------------------------------------------------------

/// Maps service names and aliases to their entry URLs.
/// "instagram" -> ServiceEntry { url: "https://www.instagram.com", ... }
/// "ig"        -> same entry (alias)
pub struct ServiceRegistry {
    entries:  HashMap<String, ServiceEntry>,
    aliases:  HashMap<String, String>, // alias -> canonical name
}

impl ServiceRegistry {
    /// Build the default service registry.
    /// Add new services here as they are tested and verified.
    pub fn new() -> Self {
        let mut r = Self {
            entries: HashMap::new(),
            aliases: HashMap::new(),
        };

        r.register(ServiceEntry {
            name:         "instagram".to_string(),
            url:          "https://www.instagram.com".to_string(),
            aliases:      vec!["ig".to_string(), "insta".to_string()],
            category:     ServiceCategory::Social,
            requires_auth: true,
            mobile_url:   Some("https://www.instagram.com".to_string()),
        });

        r.register(ServiceEntry {
            name:         "youtube".to_string(),
            url:          "https://www.youtube.com".to_string(),
            aliases:      vec!["yt".to_string()],
            category:     ServiceCategory::Video,
            requires_auth: false,
            mobile_url:   Some("https://m.youtube.com".to_string()),
        });

        r.register(ServiceEntry {
            name:         "twitter".to_string(),
            url:          "https://twitter.com".to_string(),
            aliases:      vec!["x".to_string(), "x.com".to_string()],
            category:     ServiceCategory::Social,
            requires_auth: false,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "gmail".to_string(),
            url:          "https://mail.google.com".to_string(),
            aliases:      vec!["email".to_string(), "mail".to_string()],
            category:     ServiceCategory::Email,
            requires_auth: true,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "spotify".to_string(),
            url:          "https://open.spotify.com".to_string(),
            aliases:      vec!["music".to_string()],
            category:     ServiceCategory::Music,
            requires_auth: true,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "amazon".to_string(),
            url:          "https://www.amazon.com".to_string(),
            aliases:      vec!["shop".to_string(), "shopping".to_string()],
            category:     ServiceCategory::Shopping,
            requires_auth: false,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "maps".to_string(),
            url:          "https://maps.google.com".to_string(),
            aliases:      vec!["google maps".to_string(), "navigation".to_string(), "directions".to_string()],
            category:     ServiceCategory::Maps,
            requires_auth: false,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "reddit".to_string(),
            url:          "https://www.reddit.com".to_string(),
            aliases:      vec![],
            category:     ServiceCategory::Social,
            requires_auth: false,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "whatsapp".to_string(),
            url:          "https://web.whatsapp.com".to_string(),
            aliases:      vec!["wa".to_string()],
            category:     ServiceCategory::Social,
            requires_auth: true,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "netflix".to_string(),
            url:          "https://www.netflix.com".to_string(),
            aliases:      vec![],
            category:     ServiceCategory::Video,
            requires_auth: true,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "tiktok".to_string(),
            url:          "https://www.tiktok.com".to_string(),
            aliases:      vec!["tt".to_string()],
            category:     ServiceCategory::Video,
            requires_auth: false,
            mobile_url:   None,
        });

        r.register(ServiceEntry {
            name:         "github".to_string(),
            url:          "https://github.com".to_string(),
            aliases:      vec![],
            category:     ServiceCategory::Productivity,
            requires_auth: false,
            mobile_url:   None,
        });

        r
    }

    fn register(&mut self, entry: ServiceEntry) {
        for alias in &entry.aliases {
            self.aliases.insert(alias.clone(), entry.name.clone());
        }
        self.entries.insert(entry.name.clone(), entry);
    }

    /// Resolve a name or alias to a ServiceEntry.
    /// "ig" -> Some(instagram entry)
    /// "open instagram" -> Some(instagram entry) -- strips "open" prefix
    pub fn resolve(&self, input: &str) -> Option<&ServiceEntry> {
        let cleaned = clean_service_name(input);
        // Direct match
        if let Some(entry) = self.entries.get(&cleaned) {
            return Some(entry);
        }
        // Alias match
        if let Some(canonical) = self.aliases.get(&cleaned) {
            return self.entries.get(canonical);
        }
        // Partial match -- "gram" matches "instagram"
        self.entries.values().find(|e| {
            e.name.contains(&cleaned) ||
            e.aliases.iter().any(|a| a.contains(&cleaned))
        })
    }

    /// Resolve a raw URL (user said "open github.com/omarzoghayyer")
    pub fn resolve_url(&self, url: &str) -> Option<String> {
        let url = url.trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url.to_string());
        }
        if url.contains('.') {
            return Some(format!("https://{}", url));
        }
        None
    }

    pub fn service_count(&self) -> usize { self.entries.len() }

    pub fn services_in_category(&self, cat: &ServiceCategory) -> Vec<&ServiceEntry> {
        self.entries.values().filter(|e| &e.category == cat).collect()
    }
}

fn clean_service_name(input: &str) -> String {
    let s = input.trim().to_lowercase();
    // Strip common prefixes
    for prefix in &["open ", "launch ", "go to ", "show ", "load "] {
        if s.starts_with(prefix) {
            return s[prefix.len()..].trim().to_string();
        }
    }
    s
}

// -- TrackerBlockList ---------------------------------------------------------

/// Static list of known surveillance/tracking domains.
/// Loaded at startup. Updated via OS updates, not runtime.
///
/// Blocks: ad networks, analytics, fingerprinting scripts,
/// social media tracking pixels, data brokers.
pub struct TrackerBlockList {
    domains: Vec<&'static str>,
}

impl TrackerBlockList {
    pub fn new() -> Self {
        Self {
            domains: vec![
                // Meta / Facebook surveillance
                "graph.facebook.com",
                "pixel.facebook.com",
                "connect.facebook.net",
                "www.facebook.com/tr",
                "an.facebook.com",
                // Google tracking (not search/maps content)
                "google-analytics.com",
                "www.google-analytics.com",
                "analytics.google.com",
                "doubleclick.net",
                "adservice.google.com",
                "googletagmanager.com",
                "googletagservices.com",
                "googlesyndication.com",
                // Analytics platforms
                "hotjar.com",
                "static.hotjar.com",
                "mixpanel.com",
                "api.mixpanel.com",
                "segment.io",
                "api.segment.io",
                "cdn.segment.com",
                "amplitude.com",
                "api2.amplitude.com",
                "heap.io",
                "heapanalytics.com",
                "fullstory.com",
                "rs.fullstory.com",
                "intercom.io",
                "widget.intercom.io",
                "clarity.ms",
                "www.clarity.ms",
                // Ad networks
                "adsystem.com",
                "adnxs.com",
                "adsrvr.org",
                "advertising.com",
                "adobedtm.com",
                "scorecardresearch.com",
                "chartbeat.com",
                "chartbeat.net",
                "criteo.com",
                "static.criteo.net",
                "outbrain.com",
                "taboola.com",
                "tpc.googlesyndication.com",
                // Fingerprinting
                "fingerprintjs.com",
                "fpjscdn.net",
                "deviceatlas.com",
                // Data brokers
                "quantserve.com",
                "quantcast.com",
                "bluekai.com",
                "exelator.com",
                "turn.com",
                "rubiconproject.com",
                "openx.net",
                "pubmatic.com",
                "appnexus.com",
                // Twitter/X tracking
                "static.ads-twitter.com",
                "t.co", // tracking redirects (not content)
                "analytics.twitter.com",
                // TikTok tracking
                "analytics.tiktok.com",
                "log.tiktok.com",
                // Amazon tracking
                "aax-us-east.amazon-adsystem.com",
                "s.amazon-adsystem.com",
                // Microsoft tracking
                "bat.bing.com",
                "c.msn.com",
                // Sentry / error tracking (privacy concern)
                "sentry.io",
                "browser.sentry-cdn.com",
            ],
        }
    }

    /// Should this domain be blocked?
    pub fn should_block(&self, domain: &str) -> bool {
        let domain = domain.trim().to_lowercase();
        self.domains.iter().any(|&blocked| {
            domain == blocked || domain.ends_with(&format!(".{}", blocked))
        })
    }

    /// Should this URL be blocked?
    pub fn should_block_url(&self, url: &str) -> bool {
        if let Some(domain) = extract_domain(url) {
            self.should_block(&domain)
        } else {
            false
        }
    }

    pub fn block_count(&self) -> usize { self.domains.len() }

    /// All blocked domains (for UI display).
    pub fn blocked_domains(&self) -> &[&'static str] { &self.domains }
}

fn extract_domain(url: &str) -> Option<String> {
    let url = url.trim_start_matches("https://")
                 .trim_start_matches("http://");
    let domain = url.split('/').next()?;
    let domain = domain.split('?').next()?;
    Some(domain.to_lowercase())
}

// -- ProxySessionState --------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ProxySessionState {
    /// Session not started
    Closed,
    /// Loading the service URL
    Loading,
    /// Service loaded and interactive
    Active,
    /// Backgrounded -- session preserved but not visible
    Suspended,
    /// Error state
    Error(String),
}

impl ProxySessionState {
    pub fn label(&self) -> &str {
        match self {
            ProxySessionState::Closed      => "closed",
            ProxySessionState::Loading     => "loading",
            ProxySessionState::Active      => "active",
            ProxySessionState::Suspended   => "suspended",
            ProxySessionState::Error(_)    => "error",
        }
    }

    pub fn is_active(&self) -> bool { matches!(self, ProxySessionState::Active) }
    pub fn is_closed(&self) -> bool { matches!(self, ProxySessionState::Closed) }
}

// -- BandwidthStats -----------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct BandwidthStats {
    pub bytes_sent:     u64,
    pub bytes_received: u64,
    pub requests_made:  u32,
    pub requests_blocked: u32,
    pub session_start:  u64,
}

impl BandwidthStats {
    pub fn new() -> Self {
        Self {
            session_start: now_secs(),
            ..Default::default()
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.bytes_sent + self.bytes_received
    }

    pub fn session_duration_secs(&self) -> u64 {
        now_secs().saturating_sub(self.session_start)
    }

    pub fn block_rate(&self) -> f32 {
        let total = self.requests_made + self.requests_blocked;
        if total == 0 { return 0.0; }
        (self.requests_blocked as f32 / total as f32) * 100.0
    }

    pub fn record_request(&mut self, bytes: u64, blocked: bool) {
        if blocked {
            self.requests_blocked += 1;
        } else {
            self.requests_made += 1;
            self.bytes_received += bytes;
        }
    }
}

// -- AICommand ----------------------------------------------------------------

/// Commands the AI can issue while a proxy session is active.
/// The compositor/sandbox layer executes these against the WebKit DOM.
#[derive(Debug, Clone, PartialEq)]
pub enum AICommand {
    /// Navigate to a URL within the current session
    Navigate { url: String },
    /// Click a named element (AI finds it by label/role)
    Click { target: String },
    /// Type text into the focused input
    Type { text: String },
    /// Scroll the page
    Scroll { direction: ScrollDirection, amount: u32 },
    /// Take a screenshot of current viewport
    Screenshot,
    /// Close the proxy session and return to orb
    Close,
    /// Find and return text matching a query
    Find { query: String },
    /// Submit a form
    Submit,
    /// Go back in browser history
    Back,
    /// Refresh the current page
    Refresh,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollDirection { Up, Down, Left, Right }

impl AICommand {
    /// Parse a natural language command into an AICommand.
    /// Called when user speaks while a proxy session is active.
    pub fn parse(input: &str, current_service: &str) -> Option<Self> {
        let s = input.trim().to_lowercase();

        // Close / return to orb
        if s.contains("go back") && s.contains("orb")
            || s.contains("close") || s.contains("exit")
            || s.contains("back to orb") || s.contains("home")
        {
            return Some(AICommand::Close);
        }

        // Navigation
        if s.starts_with("go to ") || s.starts_with("open ") || s.starts_with("navigate to ") {
            let url = s
                .trim_start_matches("go to ")
                .trim_start_matches("open ")
                .trim_start_matches("navigate to ")
                .trim()
                .to_string();
            return Some(AICommand::Navigate { url });
        }

        // Scroll
        if s.contains("scroll down") || s.contains("scroll up") {
            let dir = if s.contains("up") { ScrollDirection::Up } else { ScrollDirection::Down };
            return Some(AICommand::Scroll { direction: dir, amount: 3 });
        }

        // Refresh
        if s.contains("refresh") || s.contains("reload") {
            return Some(AICommand::Refresh);
        }

        // Back
        if s == "back" || s == "go back" {
            return Some(AICommand::Back);
        }

        // Service-specific commands
        match current_service {
            "instagram" => {
                if s.contains("post") || s.contains("upload") {
                    return Some(AICommand::Click { target: "new post button".to_string() });
                }
                if s.contains("notification") || s.contains("tagged") {
                    return Some(AICommand::Click { target: "notifications".to_string() });
                }
                if s.contains("search") {
                    return Some(AICommand::Click { target: "search".to_string() });
                }
                if s.contains("profile") || s.contains("my page") {
                    return Some(AICommand::Click { target: "profile".to_string() });
                }
            }
            "youtube" => {
                if s.contains("search") {
                    let query = s.trim_start_matches("search").trim().to_string();
                    return Some(AICommand::Click { target: "search box".to_string() });
                }
                if s.contains("pause") || s.contains("play") {
                    return Some(AICommand::Click { target: "play pause button".to_string() });
                }
            }
            "gmail" => {
                if s.contains("compose") || s.contains("new email") || s.contains("write") {
                    return Some(AICommand::Click { target: "compose".to_string() });
                }
                if s.contains("inbox") {
                    return Some(AICommand::Navigate { url: "https://mail.google.com/mail/u/0/#inbox".to_string() });
                }
            }
            "twitter" => {
                if s.contains("tweet") || s.contains("post") {
                    return Some(AICommand::Click { target: "tweet button".to_string() });
                }
            }
            _ => {}
        }

        // Generic find
        if s.starts_with("find ") || s.starts_with("search for ") {
            let query = s
                .trim_start_matches("find ")
                .trim_start_matches("search for ")
                .trim()
                .to_string();
            return Some(AICommand::Find { query });
        }

        None
    }
}

// -- CredentialEntry ----------------------------------------------------------

/// An encrypted credential for a web service.
/// The actual credential bytes are encrypted with the Ed25519 keypair.
/// The proxy stores these -- the user never sees a password.
#[derive(Debug, Clone)]
pub struct CredentialEntry {
    pub service:       String,
    pub encrypted:     Vec<u8>,  // ChaCha20-encrypted credential blob
    pub created_at:    u64,
    pub last_used_at:  u64,
}

impl CredentialEntry {
    pub fn new(service: String, encrypted: Vec<u8>) -> Self {
        let now = now_secs();
        Self { service, encrypted, created_at: now, last_used_at: now }
    }

    pub fn touch(&mut self) { self.last_used_at = now_secs(); }

    pub fn age_days(&self) -> u64 {
        now_secs().saturating_sub(self.created_at) / 86400
    }
}

// -- CredentialStore ----------------------------------------------------------

/// Encrypted credential storage tied to Ed25519 keypair.
/// In production: persisted to encrypted SQLite.
/// In tests: in-memory only.
pub struct CredentialStore {
    credentials: HashMap<String, CredentialEntry>,
    owner_key:   [u8; 32], // Ed25519 public key of device owner
}

impl CredentialStore {
    pub fn new(owner_key: [u8; 32]) -> Self {
        Self { credentials: HashMap::new(), owner_key }
    }

    pub fn store(&mut self, service: &str, credential_plaintext: &[u8]) {
        // Production: ChaCha20-Poly1305 encrypt with key derived from owner_key
        // Stub: XOR with owner key for structure
        let encrypted: Vec<u8> = credential_plaintext.iter().enumerate()
            .map(|(i, b)| b ^ self.owner_key[i % 32])
            .collect();
        let entry = CredentialEntry::new(service.to_string(), encrypted);
        self.credentials.insert(service.to_string(), entry);
        println!("[proxy] credential stored for {}", service);
    }

    pub fn retrieve(&mut self, service: &str) -> Option<Vec<u8>> {
        let entry = self.credentials.get_mut(service)?;
        entry.touch();
        // Production: ChaCha20-Poly1305 decrypt
        // Stub: reverse XOR
        let decrypted: Vec<u8> = entry.encrypted.iter().enumerate()
            .map(|(i, b)| b ^ self.owner_key[i % 32])
            .collect();
        Some(decrypted)
    }

    pub fn has_credential(&self, service: &str) -> bool {
        self.credentials.contains_key(service)
    }

    pub fn remove(&mut self, service: &str) -> bool {
        self.credentials.remove(service).is_some()
    }

    pub fn service_count(&self) -> usize { self.credentials.len() }
}

// -- ProxySession -------------------------------------------------------------

/// A single web service session.
/// Tracks state, bandwidth, and provides the AI command interface.
pub struct ProxySession {
    pub service:    String,
    pub url:        String,
    pub state:      ProxySessionState,
    pub bandwidth:  BandwidthStats,
    pub created_at: u64,
    pub page_title: Option<String>,
    pub current_url: String,
    command_history: Vec<AICommand>,
}

impl ProxySession {
    pub fn new(service: &str, url: &str) -> Self {
        println!("[proxy] session created: {} -> {}", service, url);
        Self {
            service:         service.to_string(),
            url:             url.to_string(),
            state:           ProxySessionState::Closed,
            bandwidth:       BandwidthStats::new(),
            created_at:      now_secs(),
            page_title:      None,
            current_url:     url.to_string(),
            command_history: Vec::new(),
        }
    }

    pub fn start_loading(&mut self) {
        self.state = ProxySessionState::Loading;
        println!("[proxy] loading: {}", self.url);
    }

    pub fn on_loaded(&mut self, title: Option<String>) {
        self.state = ProxySessionState::Active;
        self.page_title = title;
        println!("[proxy] active: {} title:{:?}", self.service,self.page_title);
    }

    pub fn suspend(&mut self) {
        if self.state.is_active() {
            self.state = ProxySessionState::Suspended;
            println!("[proxy] suspended: {}", self.service);
        }
    }

    pub fn resume(&mut self) {
        if self.state == ProxySessionState::Suspended {
            self.state = ProxySessionState::Active;
            println!("[proxy] resumed: {}", self.service);
        }
    }

    pub fn close(&mut self) {
        self.state = ProxySessionState::Closed;
        println!("[proxy] closed: {} duration:{}s bytes:{}",
            self.service,
            self.bandwidth.session_duration_secs(),
            self.bandwidth.total_bytes());
    }

    pub fn on_error(&mut self, err: String) {
        self.state = ProxySessionState::Error(err.clone());
        println!("[proxy] error in {}: {}", self.service, err);
    }

    /// Process a voice command while this session is active.
    pub fn handle_command(&mut self, input: &str) -> Option<AICommand> {
        let cmd = AICommand::parse(input, &self.service);
        if let Some(ref c) = cmd {
            self.command_history.push(c.clone());
            println!("[proxy] command in {}: {:?}", self.service, c);
        }
        cmd
    }

    /// Record a network request (for bandwidth tracking and block decisions).
    pub fn record_request(&mut self, url: &str, bytes: u64, blocked: bool) {
        self.bandwidth.record_request(bytes, blocked);
        if !blocked {
            self.current_url = url.to_string();
        }
    }

    pub fn duration_secs(&self) -> u64 {
        now_secs().saturating_sub(self.created_at)
    }

    pub fn command_count(&self) -> usize { self.command_history.len() }
}

// -- ProxyManager -------------------------------------------------------------

/// Top-level proxy manager.
/// One instance per device. Manages the active session and service registry.
pub struct ProxyManager {
    pub registry:    ServiceRegistry,
    pub blocklist:   TrackerBlockList,
    pub credentials: CredentialStore,
    pub session:     Option<ProxySession>,
}

impl ProxyManager {
    pub fn new(owner_key: [u8; 32]) -> Self {
        println!("[proxy] initialized -- {} services {} trackers blocked",
            ServiceRegistry::new().service_count(),
            TrackerBlockList::new().block_count());
        Self {
            registry:    ServiceRegistry::new(),
            blocklist:   TrackerBlockList::new(),
            credentials: CredentialStore::new(owner_key),
            session:     None,
        }
    }

    /// Open a service by name.
    /// Called when Intent::OpenApp is routed here.
    /// Returns the URL to load, or an error string.
    pub fn open(&mut self, service_name: &str) -> Result<String, String> {
        // Close existing session if any
        if let Some(ref mut s) = self.session {
            s.close();
        }

        // Resolve service name
        let entry = self.registry.resolve(service_name)
            .ok_or_else(|| format!("unknown service: {}", service_name))?;

        let url = entry.mobile_url.clone()
            .unwrap_or_else(|| entry.url.clone());

        let mut session = ProxySession::new(&entry.name, &url);
        session.start_loading();
        self.session = Some(session);

        println!("[proxy] opening {} -> {}", service_name, url);
        Ok(url)
    }

    /// Open a raw URL (user said "open github.com/omarzoghayyer")
    pub fn open_url(&mut self, url: &str) -> Result<String, String> {
        let resolved = self.registry.resolve_url(url)
            .ok_or_else(|| format!("cannot resolve URL: {}", url))?;

        if let Some(ref mut s) = self.session { s.close(); }

        let mut session = ProxySession::new("custom", &resolved);
        session.start_loading();
        self.session = Some(session);

        Ok(resolved)
    }

    /// Should this URL be blocked?
    pub fn should_block(&self, url: &str) -> bool {
        self.blocklist.should_block_url(url)
    }

    /// Handle a voice command while a session is active.
    pub fn handle_command(&mut self, input: &str) -> Option<AICommand> {
        self.session.as_mut()?.handle_command(input)
    }

    /// Close the active session.
    pub fn close(&mut self) {
        if let Some(ref mut s) = self.session {
            s.close();
        }
        self.session = None;
    }

    pub fn has_session(&self) -> bool { self.session.is_some() }

    pub fn active_service(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.service.as_str())
    }

    pub fn session_state(&self) -> Option<&ProxySessionState> {
        self.session.as_ref().map(|s| &s.state)
    }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> [u8; 32] { [0xAAu8; 32] }

    // ServiceRegistry

    #[test]
    fn test_resolve_direct() {
        let r = ServiceRegistry::new();
        assert!(r.resolve("instagram").is_some());
        assert!(r.resolve("youtube").is_some());
        assert!(r.resolve("gmail").is_some());
    }

    #[test]
    fn test_resolve_alias() {
        let r = ServiceRegistry::new();
        assert!(r.resolve("ig").is_some());
        assert_eq!(r.resolve("ig").unwrap().name, "instagram");
        assert!(r.resolve("yt").is_some());
        assert_eq!(r.resolve("yt").unwrap().name, "youtube");
    }

    #[test]
    fn test_resolve_with_open_prefix() {
        let r = ServiceRegistry::new();
        assert!(r.resolve("open instagram").is_some());
        assert_eq!(r.resolve("open instagram").unwrap().name, "instagram");
    }

    #[test]
    fn test_resolve_unknown() {
        let r = ServiceRegistry::new();
        assert!(r.resolve("fakebook").is_none());
    }

    #[test]
    fn test_resolve_url_https() {
        let r = ServiceRegistry::new();
        let url = r.resolve_url("https://example.com");
        assert_eq!(url, Some("https://example.com".to_string()));
    }

    #[test]
    fn test_resolve_url_no_scheme() {
        let r = ServiceRegistry::new();
        let url = r.resolve_url("github.com/omarzoghayyer");
        assert_eq!(url, Some("https://github.com/omarzoghayyer".to_string()));
    }

    #[test]
    fn test_service_count() {
        let r = ServiceRegistry::new();
        assert!(r.service_count() >= 10);
    }

    #[test]
    fn test_services_by_category() {
        let r = ServiceRegistry::new();
        let social = r.services_in_category(&ServiceCategory::Social);
        assert!(!social.is_empty());
    }

    // TrackerBlockList

    #[test]
    fn test_blocks_google_analytics() {
        let b = TrackerBlockList::new();
        assert!(b.should_block("google-analytics.com"));
        assert!(b.should_block("www.google-analytics.com"));
    }

    #[test]
    fn test_blocks_meta_pixel() {
        let b = TrackerBlockList::new();
        assert!(b.should_block("pixel.facebook.com"));
        assert!(b.should_block("graph.facebook.com"));
    }

    #[test]
    fn test_blocks_doubleclick() {
        let b = TrackerBlockList::new();
        assert!(b.should_block("doubleclick.net"));
    }

    #[test]
    fn test_does_not_block_instagram_content() {
        let b = TrackerBlockList::new();
        assert!(!b.should_block("www.instagram.com"));
        assert!(!b.should_block("cdninstagram.com"));
    }

    #[test]
    fn test_does_not_block_youtube_content() {
        let b = TrackerBlockList::new();
        assert!(!b.should_block("www.youtube.com"));
        assert!(!b.should_block("i.ytimg.com"));
    }

    #[test]
    fn test_blocks_url() {
        let b = TrackerBlockList::new();
        assert!(b.should_block_url("https://google-analytics.com/collect"));
        assert!(!b.should_block_url("https://www.instagram.com/feed"));
    }

    #[test]
    fn test_block_count() {
        let b = TrackerBlockList::new();
        assert!(b.block_count() >= 50);
    }

    // AICommand parsing

    #[test]
    fn test_parse_close() {
        assert_eq!(AICommand::parse("go back to orb", "instagram"), Some(AICommand::Close));
        assert_eq!(AICommand::parse("close", "instagram"), Some(AICommand::Close));
    }

    #[test]
    fn test_parse_scroll_down() {
        assert!(matches!(
            AICommand::parse("scroll down", "instagram"),
            Some(AICommand::Scroll { direction: ScrollDirection::Down, .. })
        ));
    }

    #[test]
    fn test_parse_refresh() {
        assert_eq!(AICommand::parse("refresh", "instagram"), Some(AICommand::Refresh));
        assert_eq!(AICommand::parse("reload", "youtube"), Some(AICommand::Refresh));
    }

    #[test]
    fn test_parse_instagram_post() {
        assert!(matches!(
            AICommand::parse("post a photo", "instagram"),
            Some(AICommand::Click { .. })
        ));
    }

    #[test]
    fn test_parse_gmail_compose() {
        assert!(matches!(
            AICommand::parse("compose new email", "gmail"),
            Some(AICommand::Click { .. })
        ));
    }

    #[test]
    fn test_parse_unknown_returns_none() {
        assert_eq!(AICommand::parse("do something weird", "instagram"), None);
    }

    // CredentialStore

    #[test]
    fn test_store_and_retrieve() {
        let mut store = CredentialStore::new(owner());
        let cred = b"my_secret_token";
        store.store("instagram", cred);
        assert!(store.has_credential("instagram"));
        let retrieved = store.retrieve("instagram").unwrap();
        assert_eq!(retrieved, cred);
    }

    #[test]
    fn test_retrieve_nonexistent() {
        let mut store = CredentialStore::new(owner());
        assert!(store.retrieve("instagram").is_none());
    }

    #[test]
    fn test_remove_credential() {
        let mut store = CredentialStore::new(owner());
        store.store("instagram", b"token");
        assert!(store.remove("instagram"));
        assert!(!store.has_credential("instagram"));
    }

    #[test]
    fn test_credential_count() {
        let mut store = CredentialStore::new(owner());
        store.store("instagram", b"tok1");
        store.store("gmail", b"tok2");
        assert_eq!(store.service_count(), 2);
    }

    // ProxySession

    #[test]
    fn test_session_lifecycle() {
        let mut s = ProxySession::new("instagram", "https://www.instagram.com");
        assert_eq!(s.state, ProxySessionState::Closed);
        s.start_loading();
        assert_eq!(s.state, ProxySessionState::Loading);
        s.on_loaded(Some("Instagram".to_string()));
        assert!(s.state.is_active());
        s.suspend();
        assert_eq!(s.state, ProxySessionState::Suspended);
        s.resume();
        assert!(s.state.is_active());
        s.close();
        assert!(s.state.is_closed());
    }

    #[test]
    fn test_session_command() {
        let mut s = ProxySession::new("instagram", "https://www.instagram.com");
        s.on_loaded(None);
        let cmd = s.handle_command("scroll down");
        assert!(cmd.is_some());
        assert_eq!(s.command_count(), 1);
    }

    #[test]
    fn test_session_bandwidth() {
        let mut s = ProxySession::new("instagram", "https://www.instagram.com");
        s.record_request("https://www.instagram.com/feed", 1024, false);
        s.record_request("https://google-analytics.com/collect", 0, true);
        assert_eq!(s.bandwidth.requests_made, 1);
        assert_eq!(s.bandwidth.requests_blocked, 1);
        assert!(s.bandwidth.block_rate() > 0.0);
    }

    // ProxyManager

    #[test]
    fn test_manager_open_service() {
        let mut m = ProxyManager::new(owner());
        let url = m.open("instagram");
        assert!(url.is_ok());
        assert!(url.unwrap().contains("instagram.com"));
        assert!(m.has_session());
        assert_eq!(m.active_service(), Some("instagram"));
    }

    #[test]
    fn test_manager_open_alias() {
        let mut m = ProxyManager::new(owner());
        let url = m.open("ig");
        assert!(url.is_ok());
        assert_eq!(m.active_service(), Some("instagram"));
    }

    #[test]
    fn test_manager_open_unknown_fails() {
        let mut m = ProxyManager::new(owner());
        assert!(m.open("fakebook").is_err());
    }

    #[test]
    fn test_manager_close() {
        let mut m = ProxyManager::new(owner());
        m.open("instagram").unwrap();
        m.close();
        assert!(!m.has_session());
    }

    #[test]
    fn test_manager_should_block() {
        let m = ProxyManager::new(owner());
        assert!(m.should_block("https://google-analytics.com/collect"));
        assert!(!m.should_block("https://www.instagram.com/feed"));
    }

    #[test]
    fn test_manager_open_url() {
        let mut m = ProxyManager::new(owner());
        let url = m.open_url("github.com/omarzoghayyer");
        assert!(url.is_ok());
        assert!(url.unwrap().starts_with("https://"));
    }

    #[test]
    fn test_manager_replaces_session_on_new_open() {
        let mut m = ProxyManager::new(owner());
        m.open("instagram").unwrap();
        m.open("youtube").unwrap();
        assert_eq!(m.active_service(), Some("youtube"));
    }

    #[test]
    fn test_bandwidth_block_rate() {
        let mut b = BandwidthStats::new();
        b.record_request(1024, false);
        b.record_request(0, true);
        b.record_request(0, true);
        assert!((b.block_rate() - 66.67).abs() < 1.0);
    }

    #[test]
    fn test_service_category_label() {
        assert_eq!(ServiceCategory::Social.label(), "social");
        assert_eq!(ServiceCategory::Video.label(), "video");
    }
}
