use std::net::IpAddr;

use crate::config::Protocol;

pub struct ProbeResult {
    pub up: bool,
    pub rtt_ms: Option<f64>,
    pub ssl_duration_ms: Option<f64>,
    pub ip: IpAddr,
    pub port: Option<u16>,
    pub protocol: Protocol,
    pub target: String,
}

impl ProbeResult {
    pub fn with_extra_labels(self, extra_labels: &std::collections::HashMap<String, String>) -> LabeledProbeResult {
        LabeledProbeResult {
            inner: self,
            extra_labels: extra_labels.clone(),
        }
    }
}

pub struct LabeledProbeResult {
    pub inner: ProbeResult,
    pub extra_labels: std::collections::HashMap<String, String>,
}

impl std::fmt::Debug for ProbeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeResult")
            .field("up", &self.up)
            .field("rtt_ms", &self.rtt_ms)
            .field("ssl_duration_ms", &self.ssl_duration_ms)
            .field("ip", &self.ip)
            .field("port", &self.port)
            .field("protocol", &self.protocol)
            .field("target", &self.target)
            .finish()
    }
}

pub mod tcp;
pub mod udp;
pub mod icmp;
pub mod http;
pub mod arp;