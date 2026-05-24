use std::sync::Arc;

use clap::Parser;
use tracing::{error, info, warn};

mod config;
mod metrics;
mod probe;
mod resolver;
mod scheduler;
mod watcher;

#[derive(Parser)]
#[command(name = "uptimemaster", about = "Network uptime monitoring daemon that exposes Prometheus metrics")]
struct Cli {
    /// Path to configuration directory
    #[arg(short, long, default_value = "/config")]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::ACTIVE)
        .init();

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let cli = Cli::parse();
    let config_path = cli.config;

    info!("Loading config from: {}", config_path);

    let initial_config = match config::load_from_dir(&config_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    for warning in validate_icmp_privileges(&initial_config) {
        warn!("{}", warning);
    }

    let metrics = Arc::new(metrics::Metrics::new());
    let mut sched = scheduler::Scheduler::new(initial_config.clone(), metrics.clone());
    sched.start().await;

    let metrics_port = initial_config.general.port;
    let metrics_registry = metrics.registry();

    let server_handle = tokio::spawn(async move {
        start_metrics_server(metrics_port, metrics_registry).await;
    });

    let watcher_handle = tokio::spawn(async move {
        let mut watcher = match watcher::ConfigWatcher::new(&config_path) {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to create config watcher: {}", e);
                return;
            }
        };

        loop {
            if watcher.wait_for_change() {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                match config::load_from_dir(&config_path) {
                    Ok(new_config) => {
                        info!("Config reloaded successfully");
                        for warning in validate_icmp_privileges(&new_config) {
                            warn!("{}", warning);
                        }
                        // v1.0: config hot-reload not yet wired to scheduler; restart required for full effect
                    }
                    Err(e) => {
                        error!("Failed to reload config: {}", e);
                        continue;
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = server_handle => info!("Metrics server stopped"),
        _ = watcher_handle => info!("Config watcher stopped"),
    }
}

fn validate_icmp_privileges(config: &config::Config) -> Vec<String> {
    let mut warnings = Vec::new();

    let has_icmp = config.endpoint.iter().any(|e| e.protocol == config::Protocol::Icmp);
    if has_icmp {
        if !has_net_raw_capability() {
            warnings.push("ICMP probes configured but CAP_NET_RAW capability not detected. ICMP probes may fail. Run with --cap-add=NET_RAW or as root.".to_string());
        }
    }

    warnings
}

fn has_net_raw_capability() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        match fs::read_to_string("/proc/self/status") {
            Ok(content) => content
                .lines()
                .find(|line| line.starts_with("CapEff:"))
                .map(|line| {
                    let caps = line.split(':').nth(1).unwrap_or("").trim();
                    match u64::from_str_radix(caps, 16) {
                        Ok(v) => (v & (1 << 13)) != 0,
                        Err(_) => false,
                    }
                })
                .unwrap_or(false),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

async fn start_metrics_server(port: u16, registry: Arc<prometheus_client::registry::Registry>) {
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    let app = Router::new().route("/metrics", get(move || {
        let registry = registry.clone();
        async move {
            let mut buffer = String::new();
            match prometheus_client::encoding::text::encode(&mut buffer, &registry) {
                Ok(()) => (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
                    buffer,
                ).into_response(),
                Err(e) => {
                    tracing::error!("Failed to encode metrics: {}", e);
                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to encode metrics").into_response()
                }
            }
        }
    }));

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("Metrics server listening on {}", addr);

    if let Err(e) = axum::serve(
        tokio::net::TcpListener::bind(addr).await.unwrap(),
        app,
    )
    .await
    {
        error!("Metrics server error: {}", e);
    }
}