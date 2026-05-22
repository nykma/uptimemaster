use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

use crate::probe::LabeledProbeResult;

pub struct Metrics {
    registry: Arc<Registry>,
    um_up: Family<Vec<(String, String)>, Gauge>,
    um_request_rtt: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_ssl_duration: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
}

impl Metrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let um_up = Family::<Vec<(String, String)>, Gauge>::default();
        let um_request_rtt = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_ssl_duration = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();

        registry.register(
            "um_up",
            "Whether the probe was successful (1=up, 0=down)",
            um_up.clone(),
        );
        registry.register(
            "um_request_rtt",
            "Round-trip time in milliseconds",
            um_request_rtt.clone(),
        );
        registry.register(
            "um_ssl_duration",
            "TLS handshake duration in milliseconds",
            um_ssl_duration.clone(),
        );

        Self {
            registry: Arc::new(registry),
            um_up,
            um_request_rtt,
            um_ssl_duration,
        }
    }

    pub fn update(&self, result: &LabeledProbeResult) {
        let r = &result.inner;
        let mut labels = vec![
            ("target".to_string(), r.target.clone()),
            ("protocol".to_string(), r.protocol.to_string()),
        ];
        if !r.hide_ip_label {
            labels.push(("ip".to_string(), r.ip.to_string()));
        }
        if let Some(port) = r.port {
            labels.push(("port".to_string(), port.to_string()));
        }
        for (k, v) in &result.extra_labels {
            labels.push((k.clone(), v.clone()));
        }
        labels.sort_by(|a, b| a.0.cmp(&b.0));

        let up_gauge = self.um_up.get_or_create(&labels);
        up_gauge.set(if r.up { 1 } else { 0 });

        if let Some(rtt) = r.rtt_ms {
            let rtt_gauge = self.um_request_rtt.get_or_create(&labels);
            rtt_gauge.set(rtt);
        }

        if let Some(ssl) = r.ssl_duration_ms {
            let ssl_gauge = self.um_ssl_duration.get_or_create(&labels);
            ssl_gauge.set(ssl);
        }
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::str::FromStr;

    use super::*;
    use crate::config::Protocol;
    use crate::probe::ProbeResult;

    #[test]
    fn test_metrics_update() {
        let metrics = Metrics::new();

        let mut extra = HashMap::new();
        extra.insert("env".to_string(), "prod".to_string());

        let result = ProbeResult {
            up: true,
            rtt_ms: Some(42.5),
            ssl_duration_ms: None,
            ip: IpAddr::from_str("192.168.1.1").unwrap(),
            port: Some(443),
            protocol: Protocol::Tcp,
            target: "192.168.1.1:443".to_string(),
            hide_ip_label: false,
        }
        .with_extra_labels(&extra);

        metrics.update(&result);

        let mut labels = vec![
            ("target".to_string(), "192.168.1.1:443".to_string()),
            ("ip".to_string(), "192.168.1.1".to_string()),
            ("protocol".to_string(), "tcp".to_string()),
            ("port".to_string(), "443".to_string()),
            ("env".to_string(), "prod".to_string()),
        ];
        labels.sort_by(|a, b| a.0.cmp(&b.0));

        let up = metrics.um_up.get_or_create(&labels);
        assert_eq!(up.get(), 1);
    }
}
