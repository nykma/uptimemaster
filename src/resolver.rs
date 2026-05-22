use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_resolver::TokioResolver;
use tracing::warn;

use crate::config::{EndpointConfig, Protocol, ResolvedTarget};

/// Resolve a target string into one or more ResolvedTargets.
/// - Domain targets: DNS resolution
/// - IP targets: parsed directly
/// - MAC targets (ARP): passed through with placeholder IP
pub async fn resolve_endpoint(endpoint: &EndpointConfig) -> Vec<ResolvedTarget> {
    match endpoint.protocol {
        Protocol::Arp => {
            vec![ResolvedTarget {
                original: endpoint.target.clone(),
                ip: IpAddr::from_str("0.0.0.0").unwrap(), // ARP is L2, no IP needed
                port: None,
                protocol: Protocol::Arp,
                hide_ip_label: false,
            }]
        }
        Protocol::Icmp => resolve_icmp_target(endpoint).await,
        _ => resolve_tcp_udp_http_target(endpoint).await,
    }
}

async fn resolve_icmp_target(endpoint: &EndpointConfig) -> Vec<ResolvedTarget> {
    let target = &endpoint.target;

    if let Ok(ip) = IpAddr::from_str(target) {
        return vec![ResolvedTarget {
            original: target.clone(),
            ip,
            port: None,
            protocol: Protocol::Icmp,
            hide_ip_label: false,
        }];
    }

    let ips = resolve_hostname(target).await;
    if ips.is_empty() {
        warn!("ICMP target '{}' could not be resolved", target);
        return vec![];
    }

    if endpoint.probe_all {
        ips.into_iter()
            .map(|ip| ResolvedTarget {
                original: target.clone(),
                ip,
                port: None,
                protocol: Protocol::Icmp,
                hide_ip_label: false,
            })
            .collect()
    } else {
        vec![ResolvedTarget {
            original: target.clone(),
            ip: ips[0],
            port: None,
            protocol: Protocol::Icmp,
            hide_ip_label: true,
        }]
    }
}

async fn resolve_tcp_udp_http_target(endpoint: &EndpointConfig) -> Vec<ResolvedTarget> {
    let target = &endpoint.target;
    let default_port = endpoint.protocol.default_port();

    if matches!(endpoint.protocol, Protocol::Http | Protocol::Https) {
        return resolve_http_target(endpoint).await;
    }

    let (host, port) = parse_host_port(target, default_port);

    if let Ok(ip) = IpAddr::from_str(host) {
        return vec![ResolvedTarget {
            original: target.clone(),
            ip,
            port: Some(port),
            protocol: endpoint.protocol,
            hide_ip_label: false,
        }];
    }

    let ips = resolve_hostname(host).await;
    if ips.is_empty() {
        warn!("Target '{}' could not be resolved", target);
        return vec![];
    }

    if endpoint.probe_all {
        ips.into_iter()
            .map(|ip| ResolvedTarget {
                original: target.clone(),
                ip,
                port: Some(port),
                protocol: endpoint.protocol,
                hide_ip_label: false,
            })
            .collect()
    } else {
        vec![ResolvedTarget {
            original: target.clone(),
            ip: ips[0],
            port: Some(port),
            protocol: endpoint.protocol,
            hide_ip_label: true,
        }]
    }
}

async fn resolve_http_target(endpoint: &EndpointConfig) -> Vec<ResolvedTarget> {
    let target = &endpoint.target;

    let host = extract_host_from_url(target);
    let port = extract_port_from_url(target, endpoint.protocol.default_port());

    if let Ok(ip) = IpAddr::from_str(&host) {
        return vec![ResolvedTarget {
            original: target.clone(),
            ip,
            port: Some(port),
            protocol: endpoint.protocol,
            hide_ip_label: false,
        }];
    }

    let ips = resolve_hostname(&host).await;
    if ips.is_empty() {
        warn!("HTTP target '{}' could not be resolved", target);
        return vec![];
    }

    if endpoint.probe_all {
        ips.into_iter()
            .map(|ip| ResolvedTarget {
                original: target.clone(),
                ip,
                port: Some(port),
                protocol: endpoint.protocol,
                hide_ip_label: false,
            })
            .collect()
    } else {
        vec![ResolvedTarget {
            original: target.clone(),
            ip: ips[0],
            port: Some(port),
            protocol: endpoint.protocol,
            hide_ip_label: true,
        }]
    }
}

async fn resolve_hostname(hostname: &str) -> Vec<IpAddr> {
    let resolver = match TokioResolver::builder_tokio() {
        Ok(r) => r.build(),
        Err(e) => {
            warn!("Failed to create DNS resolver: {}", e);
            return vec![];
        }
    };

    let mut results = Vec::new();

    if let Ok(response) = resolver.ipv4_lookup(hostname).await {
        results.extend(response.into_iter().map(|r| IpAddr::V4(r.0)));
    }

    if let Ok(response) = resolver.ipv6_lookup(hostname).await {
        results.extend(response.into_iter().map(|r| IpAddr::V6(r.0)));
    }

    results
}

fn parse_host_port(target: &str, default_port: Option<u16>) -> (&str, u16) {
    if let Ok(addr) = target.parse::<SocketAddr>() {
        let port = addr.port();
        let host = target.rsplit_once(':').map(|(h, _)| h).unwrap_or(target);
        return (host, port);
    }

    // IPv6 bracket notation: [::1]:port
    if let Some(colon_pos) = target.rfind(':') {
        let host = &target[..colon_pos];
        let port_str = &target[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            return (host, port);
        }
    }

    (target, default_port.unwrap_or(0))
}

fn extract_host_from_url(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    host_port
        .split(':')
        .next()
        .unwrap_or(host_port)
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string()
}

fn extract_port_from_url(url: &str, default_port: Option<u16>) -> u16 {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    if let Some(colon_pos) = host_port.rfind(':') {
        let port_str = &host_port[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            return port;
        }
    }

    if url.starts_with("https://") {
        443
    } else {
        default_port.unwrap_or(80)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_port_with_port() {
        let (host, port) = parse_host_port("192.168.1.1:443", None);
        assert_eq!(host, "192.168.1.1");
        assert_eq!(port, 443);
    }

    #[test]
    fn test_parse_host_port_without_port() {
        let (host, port) = parse_host_port("192.168.1.1", Some(80));
        assert_eq!(host, "192.168.1.1");
        assert_eq!(port, 80);
    }

    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(extract_host_from_url("https://example.com:443/path"), "example.com");
        assert_eq!(extract_host_from_url("http://192.168.1.1/health"), "192.168.1.1");
    }

    #[test]
    fn test_extract_port_from_url() {
        assert_eq!(extract_port_from_url("https://example.com/path", Some(443)), 443);
        assert_eq!(extract_port_from_url("https://example.com:8443/path", Some(443)), 8443);
        assert_eq!(extract_port_from_url("http://example.com/path", Some(80)), 80);
    }
}