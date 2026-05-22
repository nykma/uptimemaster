use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::config::Protocol;
use crate::probe::ProbeResult;

pub async fn probe_udp(ip: IpAddr, port: u16, timeout_duration: Duration, target: String) -> ProbeResult {
    let start = std::time::Instant::now();

    let bind_addr: SocketAddr = if ip.is_ipv6() {
        "[::]:0".parse().unwrap()
    } else {
        "0.0.0.0:0".parse().unwrap()
    };

    let sock = match UdpSocket::bind(bind_addr) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("UDP bind failed: {}", e);
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(port),
                protocol: Protocol::Udp,
                target,
                hide_ip_label: false,
            };
        }
    };

    let dest = SocketAddr::new(ip, port);

    // Send an empty datagram; the user can configure a payload in the future.
    if let Err(e) = sock.send_to(&[], dest) {
        tracing::debug!("UDP send failed: {}", e);
        return ProbeResult {
            up: false,
            rtt_ms: None,
            ssl_duration_ms: None,
            ip,
            port: Some(port),
            protocol: Protocol::Udp,
            target,
            hide_ip_label: false,
        };
    }

    sock.set_read_timeout(Some(timeout_duration)).ok();

    let mut buf = [0u8; 64];
    match sock.recv_from(&mut buf) {
        Ok((_len, _addr)) => {
            let rtt = start.elapsed().as_secs_f64() * 1000.0;
            ProbeResult {
                up: true,
                rtt_ms: Some(rtt),
                ssl_duration_ms: None,
                ip,
                port: Some(port),
                protocol: Protocol::Udp,
                target,
                hide_ip_label: false,
            }
        }
        Err(e) => {
            tracing::debug!("UDP recv failed: {}", e);
            ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(port),
                protocol: Protocol::Udp,
                target,
                hide_ip_label: false,
            }
        }
    }
}