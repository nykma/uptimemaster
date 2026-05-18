use std::net::IpAddr;
use std::time::Duration;

use pnet_base::MacAddr;

use crate::config::Protocol;
use crate::probe::ProbeResult;

pub async fn probe_arp(mac_addr: &str, timeout_duration: Duration, target: String) -> ProbeResult {
    let start = std::time::Instant::now();

    let mac = match parse_mac_address(mac_addr) {
        Some(m) => m,
        None => {
            tracing::error!("Invalid MAC address: {}", mac_addr);
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip: "0.0.0.0".parse().unwrap(),
                port: None,
                protocol: Protocol::Arp,
                target,
            };
        }
    };

    let interface = match find_interface_for_arp() {
        Some(iface) => iface,
        None => {
            tracing::error!("No suitable network interface found for ARP probe");
            return ProbeResult {
                up: false,
                rtt_ms: None,
                ssl_duration_ms: None,
                ip: "0.0.0.0".parse().unwrap(),
                port: None,
                protocol: Protocol::Arp,
                target,
            };
        }
    };

    let result = send_arp_request(&interface, &mac, timeout_duration).await;

    let rtt = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(true) => ProbeResult {
            up: true,
            rtt_ms: Some(rtt),
            ssl_duration_ms: None,
            ip: "0.0.0.0".parse().unwrap(),
            port: None,
            protocol: Protocol::Arp,
            target,
        },
        _ => ProbeResult {
            up: false,
            rtt_ms: None,
            ssl_duration_ms: None,
            ip: "0.0.0.0".parse().unwrap(),
            port: None,
            protocol: Protocol::Arp,
            target,
        },
    }
}

fn parse_mac_address(s: &str) -> Option<MacAddr> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let bytes: Vec<u8> = parts.iter().map(|p| u8::from_str_radix(p, 16).ok()).collect::<Option<Vec<u8>>>()?;
    Some(MacAddr::new(bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]))
}

fn find_interface_for_arp() -> Option<pnet_datalink::NetworkInterface> {
    let interfaces = pnet_datalink::interfaces();
    interfaces.into_iter().find(|iface| {
        iface.is_up()
            && !iface.is_loopback()
            && iface.mac.is_some()
            && iface.ips.iter().any(|ip| matches!(ip.ip(), IpAddr::V4(_)))
    })
}

async fn send_arp_request(
    interface: &pnet_datalink::NetworkInterface,
    target_mac: &MacAddr,
    timeout_duration: Duration,
) -> Result<bool, String> {
    use pnet_datalink::Channel::Ethernet;

    let src_mac = interface.mac.unwrap_or_else(MacAddr::broadcast);

    let (mut tx, mut rx) = match pnet_datalink::channel(interface, Default::default()) {
        Ok(Ethernet(tx, rx)) => (tx, rx),
        Ok(_) => return Err("Unsupported channel type".to_string()),
        Err(e) => return Err(format!("Failed to create channel: {}", e)),
    };

    let src_ip = interface.ips.iter()
        .find(|ip| matches!(ip.ip(), IpAddr::V4(_)))
        .map(|ip| if let IpAddr::V4(v4) = ip.ip() { v4 } else { std::net::Ipv4Addr::UNSPECIFIED })
        .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);

    let mut packet_buf = [0u8; 42];
    build_arp_packet(&mut packet_buf, &src_mac, &MacAddr::broadcast(), target_mac, src_ip);

    match tx.send_to(&packet_buf, None) {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(format!("Failed to send ARP request: {}", e)),
        None => return Err("Failed to send ARP request: channel closed".to_string()),
    }

    let deadline = std::time::Instant::now() + timeout_duration;
    loop {
        if std::time::Instant::now() > deadline {
            return Ok(false);
        }

        match rx.next() {
            Ok(packet) => {
                if packet.len() < 42 {
                    continue;
                }
                if packet[12] == 0x08 && packet[13] == 0x06 {
                    if packet[20] == 0x00 && packet[21] == 0x02 {
                        let sender_mac = MacAddr::new(
                            packet[22], packet[23], packet[24],
                            packet[25], packet[26], packet[27],
                        );
                        if &sender_mac == target_mac {
                            return Ok(true);
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }
}

fn build_arp_packet(buf: &mut [u8; 42], src_mac: &MacAddr, dst_mac: &MacAddr, target_mac: &MacAddr, src_ip: std::net::Ipv4Addr) {
    buf[0..6].copy_from_slice(&[dst_mac.0, dst_mac.1, dst_mac.2, dst_mac.3, dst_mac.4, dst_mac.5]);
    buf[6..12].copy_from_slice(&[src_mac.0, src_mac.1, src_mac.2, src_mac.3, src_mac.4, src_mac.5]);
    buf[12] = 0x08;
    buf[13] = 0x06;
    buf[14] = 0x00; buf[15] = 0x01;
    buf[16] = 0x08; buf[17] = 0x00;
    buf[18] = 6;
    buf[19] = 4;
    buf[20] = 0x00; buf[21] = 0x01;
    buf[22..28].copy_from_slice(&[src_mac.0, src_mac.1, src_mac.2, src_mac.3, src_mac.4, src_mac.5]);
    let src_octets = src_ip.octets();
    buf[28..32].copy_from_slice(&src_octets);
    buf[32..38].copy_from_slice(&[target_mac.0, target_mac.1, target_mac.2, target_mac.3, target_mac.4, target_mac.5]);
    buf[38..42].copy_from_slice(&src_octets);
}
