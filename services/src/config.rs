use std::fs;

#[derive(Debug, Clone)]
pub struct SipConfig {
    pub server: String,
    pub username: String,
    pub password: String,
    pub port: u16,
}

impl SipConfig {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let contents = fs::read_to_string("config.toml")?;
        let parsed: toml::Value = toml::from_str(&contents)?;
        let sip = &parsed["sip"];
        Ok(Self {
            server:   sip["server"].as_str().unwrap().to_string(),
            username: sip["username"].as_str().unwrap().to_string(),
            password: sip["password"].as_str().unwrap().to_string(),
            port:     sip["port"].as_integer().unwrap() as u16,
        })
    }
}