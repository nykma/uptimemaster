use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::TokioResolver;
use tracing::warn;

/// Per-call timeout for DNS lookups. Prevents a stuck DNS server from
/// delaying the entire probe cycle indefinitely.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

use crate::config::{DnsConfig, DnsProtocol, IpVersion};
use crate::config::{EndpointConfig, Protocol, ResolvedTarget};

pub async fn build_resolver(dns_config: Option<&DnsConfig>) -> Option<TokioResolver> {
    let dns = match dns_config {
        None => return build_system_resolver(),
        Some(c) => c,
    };

    // Collect server list: `servers` takes precedence, then `server`.
    let server_list: Vec<String> = if let Some(ref ss) = dns.servers {
        if ss.is_empty() {
            return build_system_resolver();
        }
        ss.clone()
    } else if let Some(ref s) = dns.server {
        vec![s.clone()]
    } else {
        return build_system_resolver();
    };

    let protocol = match dns.protocol {
        Some(p) => p,
        None => {
            warn!("DNS protocol not specified, falling back to system resolver");
            return build_system_resolver();
        }
    };

    let mut group = NameServerConfigGroup::new();

    for server_str in &server_list {
        let (host, port) = parse_dns_server(server_str, &protocol);

        let socket_addr = match tokio::net::lookup_host(format!("{}:{}", host, port)).await {
            Ok(mut addrs) => match addrs.next() {
                Some(a) => a,
                None => {
                    warn!("DNS server '{}' resolved to no addresses", server_str);
                    continue;
                }
            },
            Err(e) => {
                warn!("Failed to resolve DNS server '{}': {}", server_str, e);
                continue;
            }
        };

        let server_group = match protocol {
            DnsProtocol::Udp | DnsProtocol::Tcp => {
                NameServerConfigGroup::from_ips_clear(&[socket_addr.ip()], port, true)
            }
            DnsProtocol::Dot => NameServerConfigGroup::from_ips_tls(
                &[socket_addr.ip()],
                port,
                host.to_string(),
                true,
            ),
            DnsProtocol::Doh => NameServerConfigGroup::from_ips_https(
                &[socket_addr.ip()],
                port,
                host.to_string(),
                true,
            ),
        };

        group.merge(server_group);
    }

    if group.is_empty() {
        warn!("No DNS servers could be resolved, falling back to system resolver");
        return build_system_resolver();
    }

    let resolver_config = ResolverConfig::from_parts(None, vec![], group);

    let resolver = TokioResolver::builder_with_config(
        resolver_config,
        TokioConnectionProvider::default(),
    )
    .with_options(ResolverOpts::default())
    .build();

    Some(resolver)
}

fn build_system_resolver() -> Option<TokioResolver> {
    match TokioResolver::builder_tokio() {
        Ok(r) => Some(r.build()),
        Err(e) => {
            warn!("Failed to create default DNS resolver: {}", e);
            None
        }
    }
}

fn parse_dns_server<'a>(server: &'a str, protocol: &DnsProtocol) -> (&'a str, u16) {
    match protocol {
        DnsProtocol::Doh => {
            let without_scheme = server
                .strip_prefix("https://")
                .or_else(|| server.strip_prefix("http://"))
                .unwrap_or(server);
            let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
            if let Some(colon) = host_port.rfind(':') {
                let host = &host_port[..colon];
                let port: u16 = host_port[colon + 1..].parse().unwrap_or(443);
                (host, port)
            } else {
                (host_port, 443)
            }
        }
        _ => {
            let default_port = match protocol {
                DnsProtocol::Dot => 853,
                _ => 53,
            };
            if let Some(colon) = server.rfind(':') {
                let host = &server[..colon];
                let port: u16 = server[colon + 1..].parse().unwrap_or(default_port);
                (host, port)
            } else {
                (server, default_port)
            }
        }
    }
}

pub async fn resolve_endpoint(
    resolver: &TokioResolver,
    fallback_resolver: Option<&TokioResolver>,
    endpoint: &EndpointConfig,
) -> Vec<ResolvedTarget> {
    match endpoint.protocol {
        Protocol::Arp => {
            vec![ResolvedTarget {
                original: endpoint.target.clone(),
                ip: IpAddr::from_str("0.0.0.0").unwrap(),
                port: None,
                protocol: Protocol::Arp,
                hide_ip_label: false,
            }]
        }
        Protocol::Icmp => resolve_icmp_target(resolver, fallback_resolver, endpoint).await,
        _ => resolve_tcp_udp_http_target(resolver, fallback_resolver, endpoint).await,
    }
}

async fn resolve_icmp_target(
    resolver: &TokioResolver,
    fallback_resolver: Option<&TokioResolver>,
    endpoint: &EndpointConfig,
) -> Vec<ResolvedTarget> {
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

    let ips = resolve_hostname(resolver, fallback_resolver, target, endpoint.ip_version).await;
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

async fn resolve_tcp_udp_http_target(
    resolver: &TokioResolver,
    fallback_resolver: Option<&TokioResolver>,
    endpoint: &EndpointConfig,
) -> Vec<ResolvedTarget> {
    let target = &endpoint.target;
    let default_port = endpoint.protocol.default_port();

    if matches!(endpoint.protocol, Protocol::Http | Protocol::Https) {
        return resolve_http_target(resolver, fallback_resolver, endpoint).await;
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

    let ips = resolve_hostname(resolver, fallback_resolver, host, endpoint.ip_version).await;
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

async fn resolve_http_target(
    resolver: &TokioResolver,
    fallback_resolver: Option<&TokioResolver>,
    endpoint: &EndpointConfig,
) -> Vec<ResolvedTarget> {
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

    let ips = resolve_hostname(resolver, fallback_resolver, &host, endpoint.ip_version).await;
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

async fn resolve_hostname(
    resolver: &TokioResolver,
    fallback_resolver: Option<&TokioResolver>,
    hostname: &str,
    ip_version: Option<IpVersion>,
) -> Vec<IpAddr> {
    let mut results = do_lookup(resolver, hostname, ip_version).await;

    // Fall back to system resolver if custom DNS returned nothing
    if results.is_empty()
        && let Some(fallback) = fallback_resolver
    {
        warn!(
            "Custom DNS returned no results for '{}', falling back to system resolver",
            hostname
        );
        results = do_lookup(fallback, hostname, ip_version).await;
    }

    results
}

async fn do_lookup(
    resolver: &TokioResolver,
    hostname: &str,
    ip_version: Option<IpVersion>,
) -> Vec<IpAddr> {
    let mut results = Vec::new();
    let ipv = ip_version.unwrap_or(IpVersion::Any);

    if ipv != IpVersion::V6 {
        let ipv4_fut = resolver.ipv4_lookup(hostname);
        match tokio::time::timeout(DNS_LOOKUP_TIMEOUT, ipv4_fut).await {
            Ok(Ok(response)) => {
                results.extend(response.into_iter().map(|r| IpAddr::V4(r.0)));
            }
            Ok(Err(e)) => {
                warn!("IPv4 lookup for '{}' failed: {}", hostname, e);
            }
            Err(_) => {
                warn!(
                    "IPv4 lookup for '{}' timed out after {:?}",
                    hostname, DNS_LOOKUP_TIMEOUT
                );
            }
        }
    }

    if ipv != IpVersion::V4 {
        let ipv6_fut = resolver.ipv6_lookup(hostname);
        match tokio::time::timeout(DNS_LOOKUP_TIMEOUT, ipv6_fut).await {
            Ok(Ok(response)) => {
                results.extend(response.into_iter().map(|r| IpAddr::V6(r.0)));
            }
            Ok(Err(e)) => {
                warn!("IPv6 lookup for '{}' failed: {}", hostname, e);
            }
            Err(_) => {
                warn!(
                    "IPv6 lookup for '{}' timed out after {:?}",
                    hostname, DNS_LOOKUP_TIMEOUT
                );
            }
        }
    }

    results
}

fn parse_host_port(target: &str, default_port: Option<u16>) -> (&str, u16) {
    if let Ok(addr) = target.parse::<SocketAddr>() {
        let port = addr.port();
        let host = target.rsplit_once(':').map(|(h, _)| h).unwrap_or(target);
        return (host, port);
    }

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
        assert_eq!(
            extract_host_from_url("https://example.com:443/path"),
            "example.com"
        );
        assert_eq!(
            extract_host_from_url("http://192.168.1.1/health"),
            "192.168.1.1"
        );
    }

    #[test]
    fn test_extract_port_from_url() {
        assert_eq!(
            extract_port_from_url("https://example.com/path", Some(443)),
            443
        );
        assert_eq!(
            extract_port_from_url("https://example.com:8443/path", Some(443)),
            8443
        );
        assert_eq!(
            extract_port_from_url("http://example.com/path", Some(80)),
            80
        );
    }

    #[test]
    fn test_parse_dns_server_udp_default_port() {
        let (host, port) = parse_dns_server("1.1.1.1", &DnsProtocol::Udp);
        assert_eq!(host, "1.1.1.1");
        assert_eq!(port, 53);
    }

    #[test]
    fn test_parse_dns_server_udp_custom_port() {
        let (host, port) = parse_dns_server("1.1.1.1:5353", &DnsProtocol::Udp);
        assert_eq!(host, "1.1.1.1");
        assert_eq!(port, 5353);
    }

    #[test]
    fn test_parse_dns_server_dot_default_port() {
        let (host, port) = parse_dns_server("1.1.1.1", &DnsProtocol::Dot);
        assert_eq!(host, "1.1.1.1");
        assert_eq!(port, 853);
    }

    #[test]
    fn test_parse_dns_server_doh_url() {
        let (host, port) = parse_dns_server("https://doh.pub/dns-query", &DnsProtocol::Doh);
        assert_eq!(host, "doh.pub");
        assert_eq!(port, 443);
    }

    #[test]
    fn test_parse_dns_server_tcp_default_port() {
        let (host, port) = parse_dns_server("8.8.8.8", &DnsProtocol::Tcp);
        assert_eq!(host, "8.8.8.8");
        assert_eq!(port, 53);
    }
}
