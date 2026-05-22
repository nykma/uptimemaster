use std::net::IpAddr;
use std::time::Duration;

use rand::random;
use surge_ping::Client;
use surge_ping::ICMP;
use surge_ping::PingIdentifier;
use surge_ping::PingSequence;

use crate::config::Protocol;
use crate::probe::ProbeResult;

pub async fn probe_icmp(ip: IpAddr, timeout_duration: Duration, target: String) -> ProbeResult {
    let config = match ip {
        IpAddr::V4(_) => surge_ping::Config::default(),
        IpAddr::V6(_) => surge_ping::Config::builder().kind(ICMP::V6).build(),
    };

    let client = match Client::new(&config) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to create ICMP client: {}", e);
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: None,
                protocol: Protocol::Icmp,
                target,
                hide_ip_label: false,
            };
        }
    };

    let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
    pinger.timeout(timeout_duration);

    let payload = &[0u8; 56];
    let ping_result = pinger.ping(PingSequence(0), payload).await;

    match ping_result {
        Ok((_icmp_reply, rtt)) => ProbeResult {
            up: true,
            rtt_ms: Some(rtt.as_secs_f64() * 1000.0),
            ssl_duration_ms: None,
            ip,
            port: None,
            protocol: Protocol::Icmp,
            target,
            hide_ip_label: false,
        },
        Err(e) => {
            match &e {
                surge_ping::SurgeError::IOError(io_err) => {
                    if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                        tracing::error!("ICMP probe requires CAP_NET_RAW or root privileges");
                    } else {
                        tracing::debug!("ICMP ping to {} failed: {}", ip, e);
                    }
                }
                _ => {
                    tracing::debug!("ICMP ping to {} failed: {}", ip, e);
                }
            }
            ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: None,
                protocol: Protocol::Icmp,
                target,
                hide_ip_label: false,
            }
        }
    }
}
