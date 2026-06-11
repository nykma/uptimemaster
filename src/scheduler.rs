use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, error, info, warn, Instrument};
use uuid::Uuid;

use crate::config::{Config, EndpointConfig, Protocol, ResolvedTarget};
use crate::metrics::Metrics;
use crate::probe;
use crate::resolver;
use hickory_resolver::TokioResolver;

pub struct Scheduler {
    handles: Vec<JoinHandle<()>>,
    metrics: Arc<Metrics>,
    semaphore: Arc<Semaphore>,
    config: Config,
    resolver: TokioResolver,
    /// System resolver used as fallback when custom DNS returns no results.
    fallback_resolver: Option<TokioResolver>,
}

impl Scheduler {
    pub async fn new(config: Config, metrics: Arc<Metrics>) -> Self {
        let max_concurrent = config.general.max_concurrent_probes;
        let resolver = resolver::build_resolver(config.dns.as_ref())
            .await
            .unwrap_or_else(|| {
                warn!("Failed to build custom DNS resolver, falling back to system default");
                TokioResolver::builder_tokio()
                    .expect("Failed to create fallback DNS resolver")
                    .build()
            });

        // Build a system resolver for fallback. Only needed when a custom DNS
        // server is configured (otherwise the primary resolver is already system).
        let fallback_resolver = if config.dns.is_some() {
            match TokioResolver::builder_tokio() {
                Ok(builder) => Some(builder.build()),
                Err(e) => {
                    warn!("Failed to build system fallback DNS resolver: {}", e);
                    None
                }
            }
        } else {
            None
        };

        Self {
            handles: Vec::new(),
            metrics,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            config,
            resolver,
            fallback_resolver,
        }
    }

    pub async fn start(&mut self) {
        let default_interval = self.config.general.default_interval_secs;
        let default_timeout = self.config.general.default_timeout_ms;

        for endpoint in self.config.endpoint.iter() {
            let interval = endpoint.effective_interval(default_interval);
            let timeout = endpoint.effective_timeout(default_timeout);

            let handle = spawn_probe_task(
                endpoint.clone(),
                self.metrics.clone(),
                self.semaphore.clone(),
                Duration::from_secs(interval),
                Duration::from_millis(timeout),
                self.config.general.extra_labels.clone(),
                self.resolver.clone(),
                self.fallback_resolver.clone(),
            );

            self.handles.push(handle);
        }

        info!("Started {} probe tasks", self.handles.len());
    }

    #[allow(dead_code)]
    pub async fn stop(&mut self) {
        for handle in self.handles.drain(..) {
            handle.abort();
        }
        info!("Stopped all probe tasks");
    }

    #[allow(dead_code)]
    pub async fn reload(&mut self, config: Config) {
        self.stop().await;

        // Rebuild resolver in case DNS config changed
        let resolver = resolver::build_resolver(config.dns.as_ref())
            .await
            .unwrap_or_else(|| {
                warn!("Failed to build custom DNS resolver, falling back to system default");
                TokioResolver::builder_tokio()
                    .expect("Failed to create fallback DNS resolver")
                    .build()
            });

        let fallback_resolver = if config.dns.is_some() {
            match TokioResolver::builder_tokio() {
                Ok(builder) => Some(builder.build()),
                Err(e) => {
                    warn!("Failed to build system fallback DNS resolver: {}", e);
                    None
                }
            }
        } else {
            None
        };

        self.config = config;
        self.resolver = resolver;
        self.fallback_resolver = fallback_resolver;
        self.semaphore = Arc::new(Semaphore::new(self.config.general.max_concurrent_probes));
        self.start().await;
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_probe_task(
    endpoint: EndpointConfig,
    metrics: Arc<Metrics>,
    semaphore: Arc<Semaphore>,
    interval: Duration,
    timeout: Duration,
    general_extra_labels: HashMap<String, String>,
    resolver: TokioResolver,
    fallback_resolver: Option<TokioResolver>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval_timer = time::interval(interval);
        interval_timer.tick().await;

        loop {
            interval_timer.tick().await;

            metrics.inc_active_probes();
            let probe_start = Instant::now();

            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    warn!("Semaphore closed, stopping for {}", endpoint.target);
                    metrics.dec_active_probes();
                    return;
                }
            };

            // ── DNS resolution (moved out of run_probe for lookup tracking) ──
            let targets =
                resolver::resolve_endpoint(&resolver, fallback_resolver.as_ref(), &endpoint).await;

            // Record DNS lookup status
            if endpoint.protocol != Protocol::Arp {
                let status = if targets.is_empty() {
                    "failure"
                } else {
                    "success"
                };
                metrics.record_dns_lookup(
                    status,
                    &endpoint.target,
                    &endpoint.protocol.to_string(),
                );
            }

            let results =
                run_probe_on_targets(&endpoint, timeout, targets, Some(&metrics)).await;

            let mut merged_labels = general_extra_labels.clone();
            for (k, v) in &endpoint.extra_labels {
                merged_labels.insert(k.clone(), v.clone());
            }

            for result in results {
                let labeled = result.with_extra_labels(&merged_labels);
                metrics.update(&labeled);
            }

            // Record per-endpoint probe duration
            let duration = probe_start.elapsed().as_secs_f64();
            let mut duration_labels: Vec<(String, String)> = vec![
                ("target".to_string(), endpoint.target.clone()),
                ("protocol".to_string(), endpoint.protocol.to_string()),
            ];
            for (k, v) in &merged_labels {
                duration_labels.push((k.clone(), v.clone()));
            }
            metrics.record_probe_duration(duration, &duration_labels);

            metrics.dec_active_probes();
        }
    })
}

/// Execute probes against already-resolved targets.
pub(crate) async fn run_probe_on_targets(
    endpoint: &EndpointConfig,
    timeout: Duration,
    targets: Vec<ResolvedTarget>,
    metrics: Option<&Arc<Metrics>>,
) -> Vec<probe::ProbeResult> {
    if targets.is_empty() && endpoint.protocol != Protocol::Arp {
        warn!("No resolved targets for {}", endpoint.target);
        return vec![];
    }

    let mut results = Vec::new();

    for target in targets {
        let request_id = Uuid::new_v4();
        let span = tracing::debug_span!(
            "probe",
            request_id = %request_id,
            target = %endpoint.target,
            ip = %target.ip,
            protocol = ?endpoint.protocol,
        );

        let _enter = span.enter();
        debug!("Probe start");
        drop(_enter);

        let result = async {
            match endpoint.protocol {
                Protocol::Tcp => {
                    let port = target.port.unwrap_or(80);
                    Some(
                        probe::tcp::probe_tcp(target.ip, port, timeout, target.original.clone())
                            .await,
                    )
                }
                Protocol::Udp => {
                    let port = target.port.unwrap_or(0);
                    if port == 0 {
                        error!("UDP probe requires a port: {}", endpoint.target);
                        return None;
                    }
                    Some(
                        probe::udp::probe_udp(target.ip, port, timeout, target.original.clone())
                            .await,
                    )
                }
                Protocol::Icmp => {
                    Some(
                        probe::icmp::probe_icmp(target.ip, timeout, target.original.clone()).await,
                    )
                }
                Protocol::Http | Protocol::Https => {
                    let redirect_counter = Arc::new(AtomicU64::new(0));
                    let result = probe::http::probe_http(
                        &endpoint.target,
                        target.ip,
                        target.port,
                        endpoint.method,
                        &endpoint.headers,
                        &endpoint.payload,
                        &endpoint.content_type,
                        &endpoint.effective_expected_status(),
                        &endpoint.expected_body,
                        &endpoint.expected_body_regex,
                        timeout,
                        target.original.clone(),
                        endpoint.user_agent.as_deref(),
                        endpoint.follow_redirects,
                        endpoint.effective_max_redirects(),
                        redirect_counter.clone(),
                    )
                    .await;
                    // Record redirect count if any occurred
                    let redirects =
                        redirect_counter.load(std::sync::atomic::Ordering::Relaxed);
                    if redirects > 0 {
                        if let Some(m) = metrics {
                            m.record_http_redirects(
                                redirects,
                                &endpoint.target,
                                &endpoint.protocol.to_string(),
                            );
                        }
                    }
                    Some(result)
                }
                Protocol::Arp => {
                    Some(
                        probe::arp::probe_arp(
                            &endpoint.target,
                            timeout,
                            target.original.clone(),
                        )
                        .await,
                    )
                }
            }
        }
        .instrument(span.clone())
        .await;

        let result = match result {
            Some(r) => r,
            None => continue,
        };

        let mut result = result;
        result.hide_ip_label = target.hide_ip_label;
        results.push(result);

        let _enter = span.enter();
        debug!("Probe end");
        drop(_enter);
    }

    results
}
