use std::net::IpAddr;

use serde::Serialize;

use crate::config::Protocol;

#[derive(Serialize)]
pub struct ProbeResult {
    pub up: bool,
    pub rtt_ms: Option<f64>,
    pub ssl_duration_ms: Option<f64>,
    /// Unix timestamp (seconds) when the TLS certificate expires.
    /// Only populated for HTTPS probes, None for all other protocols.
    pub cert_expiry_secs: Option<f64>,
    pub ip: IpAddr,
    pub port: Option<u16>,
    pub protocol: Protocol,
    pub target: String,
    pub hide_ip_label: bool,
}

impl ProbeResult {
    pub fn with_extra_labels(
        self,
        extra_labels: &std::collections::HashMap<String, String>,
    ) -> LabeledProbeResult {
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
            .field("cert_expiry_secs", &self.cert_expiry_secs)
            .field("ip", &self.ip)
            .field("port", &self.port)
            .field("protocol", &self.protocol)
            .field("target", &self.target)
            .field("hide_ip_label", &self.hide_ip_label)
            .finish()
    }
}

pub mod arp;
pub mod http;
pub mod icmp;
pub mod tcp;
pub mod udp;
