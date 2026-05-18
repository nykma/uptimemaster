use std::collections::HashMap;
use std::net::IpAddr;

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    pub endpoint: Vec<EndpointConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GeneralConfig {
    #[serde(default = "defaults::port")]
    pub port: u16,
    #[serde(default = "defaults::max_concurrent_probes")]
    pub max_concurrent_probes: usize,
    /// Default probe interval in seconds (used when endpoint doesn't specify one).
    #[serde(default = "defaults::default_interval_secs")]
    pub default_interval_secs: u64,
    /// Default probe timeout in milliseconds (used when endpoint doesn't specify one).
    #[serde(default = "defaults::default_timeout_ms")]
    pub default_timeout_ms: u64,
    /// Extra labels applied to ALL metrics (endpoint-level extra_labels override on conflict).
    #[serde(default)]
    pub extra_labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EndpointConfig {
    /// Target: IP, domain, or MAC address.
    pub target: String,
    pub protocol: Protocol,
    /// Only for http/https. Defaults to "get".
    #[serde(default = "defaults::http_method")]
    pub method: HttpMethod,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    /// Only for http/https. Defaults to [200].
    #[serde(default)]
    pub expected_status: Vec<u16>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Only sent with POST method.
    #[serde(default)]
    pub payload: Option<String>,
    #[serde(default = "defaults::content_type")]
    pub content_type: String,
    /// Probe all DNS-resolved IPs instead of just the first.
    #[serde(default)]
    #[allow(dead_code)]
    pub ping_all: bool,
    /// Extra labels to attach to Prometheus metrics for this endpoint.
    #[serde(default)]
    pub extra_labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
    Http,
    Https,
    Arp,
}

impl Protocol {
    pub fn default_port(&self) -> Option<u16> {
        match self {
            Protocol::Http => Some(80),
            Protocol::Https => Some(443),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn has_ssl(&self) -> bool {
        matches!(self, Protocol::Https)
    }

    #[allow(dead_code)]
    pub fn has_port(&self) -> bool {
        !matches!(self, Protocol::Icmp | Protocol::Arp)
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Tcp => write!(f, "tcp"),
            Protocol::Udp => write!(f, "udp"),
            Protocol::Icmp => write!(f, "icmp"),
            Protocol::Http => write!(f, "http"),
            Protocol::Https => write!(f, "https"),
            Protocol::Arp => write!(f, "arp"),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    Get,
    Post,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub original: String,
    pub ip: IpAddr,
    pub port: Option<u16>,
    #[allow(dead_code)]
    pub protocol: Protocol,
}

impl EndpointConfig {
    pub fn effective_timeout(&self, default: u64) -> u64 {
        self.timeout_ms.unwrap_or(default)
    }

    pub fn effective_interval(&self, default: u64) -> u64 {
        self.interval_secs.unwrap_or(default)
    }

    pub fn effective_expected_status(&self) -> Vec<u16> {
        if self.expected_status.is_empty()
            && matches!(self.protocol, Protocol::Http | Protocol::Https)
        {
            vec![200]
        } else {
            self.expected_status.clone()
        }
    }

    pub fn validate(&self) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();

        if !matches!(self.protocol, Protocol::Http | Protocol::Https)
            && self.method != HttpMethod::Get
        {
            warnings.push(format!(
                "endpoint '{}': method is only used for http/https, ignoring for {}",
                self.target, self.protocol
            ));
        }

        if self.payload.is_some() && self.method != HttpMethod::Post {
            warnings.push(format!(
                "endpoint '{}': payload is only sent with POST method",
                self.target
            ));
        }

        if !self.expected_status.is_empty()
            && !matches!(self.protocol, Protocol::Http | Protocol::Https)
        {
            warnings.push(format!(
                "endpoint '{}': expected_status is only used for http/https, ignoring for {}",
                self.target, self.protocol
            ));
        }

        if let Some(timeout) = self.timeout_ms {
            if timeout == 0 {
                return Err(format!(
                    "endpoint '{}': timeout_ms must be greater than 0",
                    self.target
                ));
            }
        }

        Ok(warnings)
    }
}

mod defaults {
    pub fn port() -> u16 {
        9191
    }
    pub fn max_concurrent_probes() -> usize {
        50
    }
    pub fn default_interval_secs() -> u64 {
        30
    }
    pub fn default_timeout_ms() -> u64 {
        5000
    }
    pub fn http_method() -> super::HttpMethod {
        super::HttpMethod::Get
    }
    pub fn content_type() -> String {
        "application/json".to_string()
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            max_concurrent_probes: defaults::max_concurrent_probes(),
            default_interval_secs: defaults::default_interval_secs(),
            default_timeout_ms: defaults::default_timeout_ms(),
            extra_labels: HashMap::new(),
        }
    }
}

pub fn load_from_file(path: &str) -> Result<Config, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config file '{}': {}", path, e))?;
    load_from_str(&content)
}

pub fn load_from_str(content: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(content)
        .map_err(|e| format!("failed to parse config: {}", e))?;

    for endpoint in &config.endpoint {
        endpoint.validate().map_err(|e| e)?;
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let config = load_from_str(
            r#"
[general]
port = 9191

[[endpoint]]
target = "192.168.1.1:443"
protocol = "tcp"
timeout_ms = 3000
"#,
        )
        .unwrap();

        assert_eq!(config.general.port, 9191);
        assert_eq!(config.endpoint.len(), 1);
        assert_eq!(config.endpoint[0].protocol, Protocol::Tcp);
    }

    #[test]
    fn test_parse_https_endpoint() {
        let config = load_from_str(
            r#"
[[endpoint]]
target = "https://api.example.com/health"
protocol = "https"
method = "post"
payload = '{"key": "value"}'
expected_status = [200, 201]
timeout_ms = 5000
"#,
        )
        .unwrap();

        assert_eq!(config.endpoint[0].protocol, Protocol::Https);
        assert_eq!(config.endpoint[0].method, HttpMethod::Post);
        assert_eq!(config.endpoint[0].expected_status, vec![200, 201]);
    }

    #[test]
    fn test_defaults() {
        let general = GeneralConfig::default();
        assert_eq!(general.port, 9191);
        assert_eq!(general.max_concurrent_probes, 50);
        assert_eq!(general.default_interval_secs, 30);
        assert_eq!(general.default_timeout_ms, 5000);
        assert!(general.extra_labels.is_empty());
    }
}