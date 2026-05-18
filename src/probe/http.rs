use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use crate::config::{HttpMethod, Protocol};
use crate::probe::ProbeResult;

pub async fn probe_http(
    target: &str,
    ip: IpAddr,
    port: Option<u16>,
    method: HttpMethod,
    headers: &HashMap<String, String>,
    payload: &Option<String>,
    content_type: &str,
    expected_status: &[u16],
    timeout_duration: Duration,
    target_label: String,
) -> ProbeResult {
    let start = std::time::Instant::now();

    let actual_port = port.unwrap_or_else(|| {
        if matches!(method, HttpMethod::Get | HttpMethod::Post) && target.starts_with("https://") {
            443
        } else {
            80
        }
    });

    let client = match build_client(ip, actual_port, timeout_duration) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to build HTTP client: {}", e);
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(actual_port),
                protocol: Protocol::Http,
                target: target_label,
            };
        }
    };

    let request = match method {
        HttpMethod::Get => client.get(target),
        HttpMethod::Post => {
            let req = client.post(target);
            if let Some(body) = payload {
                req.header("Content-Type", content_type).body(body.clone())
            } else {
                req
            }
        }
    };

    let mut request = request.timeout(timeout_duration);

    for (key, value) in headers {
        request = request.header(key.as_str(), value.as_str());
    }

    let response = request.send().await;

    let total_rtt = start.elapsed().as_secs_f64() * 1000.0;

    match response {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let up = expected_status.contains(&status);

            let protocol = if target.starts_with("https://") {
                Protocol::Https
            } else {
                Protocol::Http
            };

            let ssl_duration_ms = if protocol == Protocol::Https {
                measure_ssl_duration(ip, actual_port, target, timeout_duration).await
            } else {
                None
            };

            ProbeResult {
                up,
                rtt_ms: Some(total_rtt),
                ssl_duration_ms,
                ip,
                port: Some(actual_port),
                protocol,
                target: target_label,
            }
        }
        Err(e) => {
            tracing::debug!("HTTP request to {} failed: {}", target, e);

            let protocol = if target.starts_with("https://") {
                Protocol::Https
            } else {
                Protocol::Http
            };

            ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip,
                port: Some(actual_port),
                protocol,
                target: target_label,
            }
        }
    }
}

fn build_client(
    _ip: IpAddr,
    _port: u16,
    timeout_duration: Duration,
) -> Result<reqwest::Client, String> {
    reqwest::ClientBuilder::new()
        .timeout(timeout_duration)
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| format!("failed to build reqwest client: {}", e))
}

async fn measure_ssl_duration(
    ip: IpAddr,
    port: u16,
    target: &str,
    timeout_duration: Duration,
) -> Option<f64> {
    use std::sync::Arc;
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;

    let start = std::time::Instant::now();

    let hostname = extract_hostname(target)?;

    let addr = std::net::SocketAddr::new(ip, port);

    let tcp_stream = match tokio::time::timeout(timeout_duration, TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        _ => return None,
    };

    let root_store = rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    );

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));

    let server_name = rustls_pki_types::ServerName::try_from(hostname).ok()?;

    let tls_result = connector.connect(server_name, tcp_stream).await;

    match tls_result {
        Ok(_) => Some(start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            tracing::debug!("TLS handshake to {}:{} failed: {}", ip, port, e);
            None
        }
    }
}

fn extract_hostname(url: &str) -> Option<String> {
    let without_scheme = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host_port = without_scheme.split('/').next()?;
    let hostname = host_port.split(':').next()?;
    Some(hostname.to_string())
}
