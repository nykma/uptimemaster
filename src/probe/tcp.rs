use std::net::IpAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::Protocol;
use crate::probe::ProbeResult;

pub async fn probe_tcp(ip: IpAddr, port: u16, timeout_duration: Duration, target: String) -> ProbeResult {
    let start = std::time::Instant::now();

    let result = timeout(timeout_duration, TcpStream::connect((ip, port))).await;

    let rtt = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(Ok(_stream)) => ProbeResult {
            up: true,
            rtt_ms: Some(rtt),
            ssl_duration_ms: None,
            ip,
            port: Some(port),
            protocol: Protocol::Tcp,
            target,
            hide_ip_label: false,
        },
        Ok(Err(e)) => {
            tracing::debug!("TCP connect failed: {}", e);
            ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(port),
                protocol: Protocol::Tcp,
                target,
                hide_ip_label: false,
            }
        }
        Err(_) => {
            tracing::debug!("TCP connect timed out after {:?}", timeout_duration);
            ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(port),
                protocol: Protocol::Tcp,
                target,
                hide_ip_label: false,
            }
        }
    }
}