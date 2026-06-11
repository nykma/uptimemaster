use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;

use crate::probe::LabeledProbeResult;

struct EndpointState {
    prev_up: bool,
    consecutive_failures: u64,
}

pub struct Metrics {
    registry: Arc<Registry>,
    um_up: Family<Vec<(String, String)>, Gauge>,
    um_request_rtt: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_ssl_duration: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_tls_cert_expiry: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_probes_total: Family<Vec<(String, String)>, Counter>,

    // U1: New metrics
    um_probe_duration: Family<Vec<(String, String)>, Histogram>,
    um_request_rtt_hist: Family<Vec<(String, String)>, Histogram>,
    um_probes_active: Gauge<i64, AtomicI64>,
    um_consecutive_failures: Family<Vec<(String, String)>, Gauge>,
    um_last_state_change: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_last_success: Family<Vec<(String, String)>, Gauge<f64, AtomicU64>>,
    um_config_reloads: Counter,
    um_dns_lookups: Family<Vec<(String, String)>, Counter>,
    um_build_info: Family<Vec<(String, String)>, Gauge<i64, AtomicI64>>,

    // Internal state for state-change and consecutive-failure tracking
    state_map: Mutex<HashMap<Vec<(String, String)>, EndpointState>>,
}

impl Metrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let um_up = Family::<Vec<(String, String)>, Gauge>::default();
        let um_request_rtt = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_ssl_duration = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_tls_cert_expiry = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_probes_total = Family::<Vec<(String, String)>, Counter>::default();

        // U1: New metric families
        let um_probe_duration =
            Family::<Vec<(String, String)>, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        let um_request_rtt_hist =
            Family::<Vec<(String, String)>, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        let um_probes_active = Gauge::<i64, AtomicI64>::default();
        let um_consecutive_failures = Family::<Vec<(String, String)>, Gauge>::default();
        let um_last_state_change =
            Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_last_success = Family::<Vec<(String, String)>, Gauge<f64, AtomicU64>>::default();
        let um_config_reloads = Counter::default();
        let um_dns_lookups = Family::<Vec<(String, String)>, Counter>::default();
        let um_build_info = Family::<Vec<(String, String)>, Gauge<i64, AtomicI64>>::default();

        registry.register(
            "um_up",
            "Whether the probe was successful (1=up, 0=down)",
            um_up.clone(),
        );
        registry.register(
            "um_request_rtt",
            "Round-trip time in milliseconds (deprecated, use um_request_rtt_seconds instead)",
            um_request_rtt.clone(),
        );
        registry.register(
            "um_ssl_duration",
            "TLS handshake duration in milliseconds",
            um_ssl_duration.clone(),
        );
        registry.register(
            "um_tls_cert_expiry_seconds",
            "Unix timestamp when the TLS certificate expires (0 if not applicable or probe failed)",
            um_tls_cert_expiry.clone(),
        );
        registry.register(
            "um_probes_total",
            "Total number of probe attempts, labeled by status (success or failure)",
            um_probes_total.clone(),
        );

        // U1: Register new metrics
        registry.register(
            "um_probe_duration_seconds",
            "Duration of each probe cycle in seconds (wall-clock time from permit acquisition to result processing)",
            um_probe_duration.clone(),
        );
        registry.register(
            "um_request_rtt_seconds",
            "Round-trip time in seconds (histogram)",
            um_request_rtt_hist.clone(),
        );
        registry.register(
            "um_probes_active",
            "Number of probe tasks currently in-flight",
            um_probes_active.clone(),
        );
        registry.register(
            "um_consecutive_failures",
            "Consecutive probe failures for this endpoint (resets to 0 on success)",
            um_consecutive_failures.clone(),
        );
        registry.register(
            "um_last_state_change_timestamp_seconds",
            "Unix timestamp of the last up/down state transition",
            um_last_state_change.clone(),
        );
        registry.register(
            "um_last_success_timestamp_seconds",
            "Unix timestamp of the last successful probe",
            um_last_success.clone(),
        );
        registry.register(
            "um_config_reloads",
            "Total number of successful configuration reloads",
            um_config_reloads.clone(),
        );
        registry.register(
            "um_dns_lookups_total",
            "Total number of DNS lookups, labeled by status (success or failure)",
            um_dns_lookups.clone(),
        );
        registry.register(
            "um_build_info",
            "Build information (version, commit)",
            um_build_info.clone(),
        );

        Self {
            registry: Arc::new(registry),
            um_up,
            um_request_rtt,
            um_ssl_duration,
            um_tls_cert_expiry,
            um_probes_total,
            um_probe_duration,
            um_request_rtt_hist,
            um_probes_active,
            um_consecutive_failures,
            um_last_state_change,
            um_last_success,
            um_config_reloads,
            um_dns_lookups,
            um_build_info,
            state_map: Mutex::new(HashMap::new()),
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

        let rtt_gauge = self.um_request_rtt.get_or_create(&labels);
        rtt_gauge.set(r.rtt_ms.unwrap_or(0.0));

        // U2: Record RTT in histogram (only on successful probes with a value)
        if r.up {
            if let Some(rtt_ms) = r.rtt_ms {
                self.um_request_rtt_hist
                    .get_or_create(&labels)
                    .observe(rtt_ms / 1000.0);
            }
        }

        let ssl_gauge = self.um_ssl_duration.get_or_create(&labels);
        ssl_gauge.set(r.ssl_duration_ms.unwrap_or(0.0));

        let cert_gauge = self.um_tls_cert_expiry.get_or_create(&labels);
        cert_gauge.set(r.cert_expiry_secs.unwrap_or(0.0));

        // Counter: clone labels and add status dimension
        let status = if r.up { "success" } else { "failure" };
        let mut counter_labels = labels.clone();
        counter_labels.push(("status".to_string(), status.to_string()));
        counter_labels.sort_by(|a, b| a.0.cmp(&b.0));
        self.um_probes_total.get_or_create(&counter_labels).inc();

        // U1: Consecutive failures and state-change tracking
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let (consecutive, state_change_ts, last_success_ts) = {
            let mut state_map = self.state_map.lock().unwrap();
            let state = state_map.entry(labels.clone()).or_insert(EndpointState {
                prev_up: true,
                consecutive_failures: 0,
            });

            let changed = r.up != state.prev_up;
            if changed {
                state.prev_up = r.up;
            }

            if r.up {
                state.consecutive_failures = 0;
            } else {
                state.consecutive_failures += 1;
            }

            let consecutive = state.consecutive_failures;
            let state_change_ts = if changed { Some(now) } else { None };
            let last_success_ts = if r.up { Some(now) } else { None };

            (consecutive, state_change_ts, last_success_ts)
        };

        self.um_consecutive_failures
            .get_or_create(&labels)
            .set(consecutive as i64);

        if let Some(ts) = state_change_ts {
            self.um_last_state_change.get_or_create(&labels).set(ts);
        }

        if let Some(ts) = last_success_ts {
            self.um_last_success.get_or_create(&labels).set(ts);
        }
    }

    // ── Methods called from scheduler / main ──

    /// Record wall-clock duration of one probe cycle (from permit acquire to
    /// result processing). Called once per loop iteration in the scheduler.
    pub fn record_probe_duration(&self, duration_secs: f64, labels: &[(String, String)]) {
        let mut sorted: Vec<(String, String)> = labels.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        self.um_probe_duration.get_or_create(&sorted).observe(duration_secs);
    }

    /// Increment the in-flight probe count (called before semaphore acquire).
    pub fn inc_active_probes(&self) {
        self.um_probes_active.inc();
    }

    /// Decrement the in-flight probe count (called after probe cycle completes).
    pub fn dec_active_probes(&self) {
        self.um_probes_active.dec();
    }

    /// Increment the config reload counter (called from main on successful reload).
    pub fn inc_config_reloads(&self) {
        self.um_config_reloads.inc();
    }

    /// Record a DNS lookup attempt with the given status ("success" or "failure").
    pub fn record_dns_lookup(&self, status: &str, target: &str, protocol: &str) {
        let mut labels = vec![
            ("status".to_string(), status.to_string()),
            ("target".to_string(), target.to_string()),
            ("protocol".to_string(), protocol.to_string()),
        ];
        labels.sort_by(|a, b| a.0.cmp(&b.0));
        self.um_dns_lookups.get_or_create(&labels).inc();
    }

    /// Set build info (version + commit). Called once at startup.
    pub fn set_build_info(&self, version: &str, commit: &str) {
        let mut labels = vec![
            ("version".to_string(), version.to_string()),
            ("commit".to_string(), commit.to_string()),
        ];
        labels.sort_by(|a, b| a.0.cmp(&b.0));
        self.um_build_info.get_or_create(&labels).set(1);
    }

    /// Return the current number of in-flight probes.
    pub fn active_probes(&self) -> i64 {
        self.um_probes_active.get()
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

    /// Build a sorted label set matching what `update()` produces when
    /// `hide_ip_label: true` and `port: None`.
    fn make_labels(target: &str, protocol: &str) -> Vec<(String, String)> {
        let mut labels = vec![
            ("protocol".to_string(), protocol.to_string()),
            ("target".to_string(), target.to_string()),
        ];
        labels.sort_by(|a, b| a.0.cmp(&b.0));
        labels
    }

    fn make_result(up: bool, target: &str, protocol: Protocol) -> LabeledProbeResult {
        ProbeResult {
            up,
            rtt_ms: Some(42.5),
            ssl_duration_ms: None,
            cert_expiry_secs: None,
            ip: IpAddr::from_str("192.168.1.1").unwrap(),
            port: None,
            protocol,
            target: target.to_string(),
            hide_ip_label: true,
        }
        .with_extra_labels(&HashMap::new())
    }

    #[test]
    fn test_metrics_update() {
        let metrics = Metrics::new();

        let mut extra = HashMap::new();
        extra.insert("env".to_string(), "prod".to_string());

        let result = ProbeResult {
            up: true,
            rtt_ms: Some(42.5),
            ssl_duration_ms: None,
            cert_expiry_secs: None,
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

    #[test]
    fn test_consecutive_failures_reset_on_success() {
        let metrics = Metrics::new();

        // First probe: failure
        metrics.update(&make_result(false, "target:80", Protocol::Tcp));
        let labels = make_labels("target:80", "tcp");
        let cf = metrics.um_consecutive_failures.get_or_create(&labels);
        assert_eq!(cf.get(), 1);

        // Second probe: failure
        metrics.update(&make_result(false, "target:80", Protocol::Tcp));
        assert_eq!(cf.get(), 2);

        // Third probe: success → reset
        metrics.update(&make_result(true, "target:80", Protocol::Tcp));
        assert_eq!(cf.get(), 0);
    }

    #[test]
    fn test_state_change_timestamp_set_on_transition() {
        let metrics = Metrics::new();
        let labels = make_labels("target:80", "tcp");

        // First probe: success (initial state is up, but we simulate first probe)
        metrics.update(&make_result(true, "target:80", Protocol::Tcp));

        // Second probe: failure → state changed from up to down
        metrics.update(&make_result(false, "target:80", Protocol::Tcp));
        let ts = metrics.um_last_state_change.get_or_create(&labels);
        assert!(ts.get() > 0.0, "state change timestamp should be set");

        // Third probe: still failure → no state change
        let ts_before = ts.get();
        metrics.update(&make_result(false, "target:80", Protocol::Tcp));
        assert_eq!(ts.get(), ts_before, "timestamp should not change");
    }

    #[test]
    fn test_last_success_timestamp() {
        let metrics = Metrics::new();
        let labels = make_labels("target:80", "tcp");

        // Failed probe → no last_success
        metrics.update(&make_result(false, "target:80", Protocol::Tcp));
        let ls = metrics.um_last_success.get_or_create(&labels);
        // Should not have been set (value remains 0 if never set, or whatever default)

        // Successful probe → last_success set
        metrics.update(&make_result(true, "target:80", Protocol::Tcp));
        assert!(ls.get() > 0.0, "last success timestamp should be set");
    }

    #[test]
    fn test_active_probes_inc_dec() {
        let metrics = Metrics::new();
        assert_eq!(metrics.um_probes_active.get(), 0);

        metrics.inc_active_probes();
        metrics.inc_active_probes();
        assert_eq!(metrics.um_probes_active.get(), 2);

        metrics.dec_active_probes();
        assert_eq!(metrics.um_probes_active.get(), 1);

        metrics.dec_active_probes();
        assert_eq!(metrics.um_probes_active.get(), 0);
    }

    #[test]
    fn test_config_reloads_counter() {
        let metrics = Metrics::new();
        metrics.inc_config_reloads();
        metrics.inc_config_reloads();
        let mut buf = String::new();
        prometheus_client::encoding::text::encode(&mut buf, &metrics.registry).unwrap();
        assert!(buf.contains("um_config_reloads_total"));
    }

    #[test]
    fn test_dns_lookups_recording() {
        let metrics = Metrics::new();
        metrics.record_dns_lookup("success", "example.com", "http");
        metrics.record_dns_lookup("success", "example.com", "http");
        metrics.record_dns_lookup("failure", "nxdomain.test", "tcp");

        let mut buf = String::new();
        prometheus_client::encoding::text::encode(&mut buf, &metrics.registry).unwrap();
        assert!(buf.contains("um_dns_lookups_total"));
        assert!(buf.contains("success"));
        assert!(buf.contains("failure"));
    }

    #[test]
    fn test_build_info() {
        let metrics = Metrics::new();
        metrics.set_build_info("1.0.0", "abc123def");

        let mut buf = String::new();
        prometheus_client::encoding::text::encode(&mut buf, &metrics.registry).unwrap();
        assert!(buf.contains("um_build_info"));
        assert!(buf.contains("1.0.0"));
        assert!(buf.contains("abc123def"));
    }

    #[test]
    fn test_probe_duration_recording() {
        let metrics = Metrics::new();
        let labels = vec![
            ("protocol".to_string(), "tcp".to_string()),
            ("target".to_string(), "test:80".to_string()),
        ];
        metrics.record_probe_duration(0.042, &labels);
        metrics.record_probe_duration(0.100, &labels);

        let mut buf = String::new();
        prometheus_client::encoding::text::encode(&mut buf, &metrics.registry).unwrap();
        assert!(buf.contains("um_probe_duration_seconds_bucket"));
        assert!(buf.contains("um_probe_duration_seconds_sum"));
        assert!(buf.contains("um_probe_duration_seconds_count"));
    }
}
