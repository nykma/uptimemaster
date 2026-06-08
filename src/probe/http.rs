use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::config::{HttpMethod, Protocol};
use crate::probe::ProbeResult;

#[allow(clippy::too_many_arguments)]
pub async fn probe_http(
    target: &str,
    ip: IpAddr,
    port: Option<u16>,
    method: HttpMethod,
    headers: &HashMap<String, String>,
    payload: &Option<String>,
    content_type: &str,
    expected_status: &[u16],
    expected_body: &Option<String>,
    expected_body_regex: &Option<String>,
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

    let hostname = extract_hostname(target);
    let client = match build_client(hostname.as_deref().unwrap_or(target), ip, actual_port, timeout_duration) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to build HTTP client: {}", e);
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
            cert_expiry_secs: None,
                ip,
                port: Some(actual_port),
                protocol: Protocol::Http,
                target: target_label,
                hide_ip_label: false,
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
            let mut up = expected_status.contains(&status);

            // Body content check — only if status already passed and a body check is configured.
            // We read the body once and run both the exact match and regex check against it.
            if up && (expected_body.is_some() || expected_body_regex.is_some()) {
                match resp.text().await {
                    Ok(body) => {
                        if let Some(expected) = expected_body
                            && !body.contains(expected.as_str())
                        {
                            tracing::debug!(
                                "HTTP body check failed: expected string not found in {}",
                                target
                            );
                            up = false;
                        }
                        if up
                            && let Some(pattern) = expected_body_regex
                        {
                            match regex::Regex::new(pattern) {
                                Ok(re) => {
                                    if !re.is_match(&body) {
                                        tracing::debug!(
                                            "HTTP body check failed: regex '{}' did not match for {}",
                                            pattern, target
                                        );
                                        up = false;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Invalid regex '{}' for {} (should have been caught at config load): {}",
                                        pattern, target, e
                                    );
                                    up = false;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to read HTTP response body for {}: {}", target, e);
                        up = false;
                    }
                }
            }

            let protocol = if target.starts_with("https://") {
                Protocol::Https
            } else {
                Protocol::Http
            };

            let (ssl_duration_ms, cert_expiry_secs) = if protocol == Protocol::Https {
                match measure_tls(ip, actual_port, target, timeout_duration).await {
                    Some((duration, cert)) => (Some(duration), cert),
                    None => (None, None),
                }
            } else {
                (None, None)
            };

            ProbeResult {
                up,
                rtt_ms: Some(total_rtt),
                ssl_duration_ms,
                cert_expiry_secs,
                ip,
                port: Some(actual_port),
                protocol,
                target: target_label,
                hide_ip_label: false,
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
            cert_expiry_secs: None,
                ip,
                port: Some(actual_port),
                protocol,
                target: target_label,
                hide_ip_label: false,
            }
        }
    }
}

/// Build a reqwest Client that connects to `ip:port` but uses `host` for TLS SNI
/// and the HTTP Host header. This ensures custom DNS resolution is respected
/// and `probe_all` works correctly for HTTP/HTTPS targets.
fn build_client(
    host: &str,
    ip: IpAddr,
    port: u16,
    timeout_duration: Duration,
) -> Result<reqwest::Client, String> {
    let addr = SocketAddr::new(ip, port);
    reqwest::ClientBuilder::new()
        .resolve_to_addrs(host, &[addr])
        .timeout(timeout_duration)
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| format!("failed to build reqwest client: {}", e))
}

/// Perform a TLS handshake to measure duration and extract certificate expiry.
/// Returns `(duration_ms, cert_expiry_unix_seconds)` on success.
async fn measure_tls(
    ip: IpAddr,
    port: u16,
    target: &str,
    timeout_duration: Duration,
) -> Option<(f64, Option<f64>)> {
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

    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));

    let server_name = rustls_pki_types::ServerName::try_from(hostname).ok()?;

    let tls_result = connector.connect(server_name, tcp_stream).await;

    match tls_result {
        Ok(tls_stream) => {
            let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
            let cert_expiry = extract_cert_expiry(&tls_stream);
            Some((duration_ms, cert_expiry))
        }
        Err(e) => {
            tracing::debug!("TLS handshake to {}:{} failed: {}", ip, port, e);
            None
        }
    }
}

/// Extract the `not_after` timestamp from the leaf certificate of a TLS connection.
fn extract_cert_expiry(
    tls_stream: &tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
) -> Option<f64> {
    use x509_parser::prelude::*;

    let (_, conn) = tls_stream.get_ref();
    let certs = conn.peer_certificates()?;
    let leaf_der = certs.first()?;

    match X509Certificate::from_der(leaf_der.as_ref()) {
        Ok((_, cert)) => {
            let not_after = cert.validity().not_after;
            // x509-parser returns ASN1Time; timestamp() gives Unix epoch seconds
            Some(not_after.timestamp() as f64)
        }
        Err(e) => {
            tracing::debug!("Failed to parse peer certificate: {}", e);
            None
        }
    }
}

fn extract_hostname(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_port = without_scheme.split('/').next()?;
    let hostname = host_port.split(':').next()?;
    Some(hostname.to_string())
}
