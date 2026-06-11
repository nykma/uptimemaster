# uptimemaster

Network uptime monitoring daemon written in Rust. Regularly probes your configured network endpoints and exposes the results as [Prometheus](https://prometheus.io/) metrics.

## Quick Start (Docker Compose)

```bash
mkdir config
cp config.sample.toml config/config.toml
# edit config/config.toml to suit your needs
docker compose up -d
```

The configuration is fully described in [`config.sample.toml`](config.sample.toml). Copy it to `config/config.toml`, adjust the endpoints you want to monitor, and you are ready to go.

Prometheus metrics are available at `http://localhost:9191/metrics` (port configurable in `[general]`).

> **Note:** The container image is built from the Nix flake and published to `ghcr.io`. See [`docker-compose.yml`](docker-compose.yml) for the exact image tag and mount details.
>
> Config is loaded from a **directory** (`/config` by default). All `.toml` files in it are read in alphabetical order and merged. See [Configuration](#configuration) for details.

## Probe Types

| Protocol | Description | Target Example |
|---|---|---|
| `tcp` | TCP connect probe | `192.168.1.1:80` |
| `udp` | UDP datagram send/receive | `8.8.8.8:53` |
| `icmp` | ICMP echo (ping) | `8.8.8.8` or `example.com` |
| `http` | HTTP GET/POST, status code check | `http://example.com/api` |
| `https` | HTTPS with TLS handshake timing | `https://example.com/health` |
| `arp` | ARP request (L2, MAC address) | `aa:bb:cc:dd:ee:ff` |

HTTP probes support `GET` or `POST` methods, custom headers, JSON payloads, and expected status code validation. HTTPS probes additionally measure the TLS handshake duration.

## Important: ICMP Requires `NET_RAW`

ICMP ping uses raw sockets, which require the `CAP_NET_RAW` Linux capability. **If you use `protocol = "icmp"` for any endpoint, you must grant this capability.**

### In Docker Compose

The provided [`docker-compose.yml`](docker-compose.yml) already includes:

```yaml
cap_add:
  - NET_RAW
```

No extra steps are needed if you use the bundled Compose file.

### Running directly (without Docker)

Run as `root`, or grant the capability to the binary:

```bash
sudo setcap cap_net_raw+ep ./uptimemaster
```

If `CAP_NET_RAW` is missing, uptimemaster prints a warning at startup and ICMP probes will fail at runtime with a permission error.

### systemd Deployment

A [systemd unit template](contrib/uptimemaster.service) is provided for running uptimemaster as a service:

```bash
# Install the binary
sudo cp uptimemaster /usr/local/bin/uptimemaster
sudo setcap cap_net_raw+ep /usr/local/bin/uptimemaster

# Install config
sudo mkdir -p /etc/uptimemaster/config
sudo cp config/*.toml /etc/uptimemaster/config/

# Install and enable the service
sudo cp contrib/uptimemaster.service /etc/systemd/system/
sudo mkdir -p /var/lib/uptimemaster
sudo systemctl daemon-reload
sudo systemctl enable --now uptimemaster
```

Pre-built binaries (`uptimemaster-x86_64`, `uptimemaster-aarch64`) are attached to [GitHub Releases](https://github.com/nykma/uptimemaster/releases) for every semver tag.

## Configuration

uptimemaster reads all `.toml` files from a configuration directory (default: `/config`).

- **`config.toml`** is processed first and is the **only** file that may define `[general]` and `[dns]`. All other files must contain only `[[endpoint]]` entries.
- Other `.toml` files (e.g. `servers.toml`, `web.toml`) are read in alphabetical order and their endpoints are merged.
- Hot-reloading: changes to any `.toml` file in the directory trigger a config reload.

See [`config.sample.toml`](config.sample.toml) for all options and real-world examples. Quick overview:

```toml
# config/config.toml — the only file that can have [general] and [dns]
[general]
port = 9191
max_concurrent_probes = 50
default_interval_secs = 30
default_timeout_ms = 5000
extra_labels = { node = "my_home" }

[dns]
server = "1.1.1.1"
protocol = "udp"

[[endpoint]]
target = "192.168.1.1:80"
protocol = "tcp"

# config/servers.toml — endpoint-only file
[[endpoint]]
target = "8.8.4.4"
protocol = "icmp"

# config/web.toml — endpoint-only file
[[endpoint]]
target = "https://example.com/health"
protocol = "https"
method = "get"
expected_status = [200]
expected_body = "ok"  # optional: verify response body

# config/api.toml — with regex body check
[[endpoint]]
target = "https://api.example.com/status"
protocol = "https"
method = "get"
expected_status = [200]
expected_body_regex = '^\{"status":"healthy".*\}$'  # optional: match body against regex
```

- `[general]` — global defaults (metrics port, concurrency, probe interval, timeout). **Only allowed in `config.toml`.**
- `[dns]` — custom DNS resolver configuration (optional). **Only allowed in `config.toml`.**
- `[[endpoint]]` — one entry per target. Required fields: `target`, `protocol`. All other fields have sensible defaults.

### DNS Configuration (`[dns]`)

By default, uptimemaster uses the system DNS resolver. You can override this with a custom DNS server:

```toml
# config/config.toml
[dns]
server = "1.1.1.1"       # IP or hostname, optional :port
protocol = "udp"          # udp | tcp | dot | doh
```

| Protocol | Description | Default Port | `server` Example |
|---|---|---|---|
| `udp` | Standard DNS over UDP | 53 | `1.1.1.1` or `8.8.8.8:53` |
| `tcp` | DNS over TCP | 53 | `8.8.8.8:53` |
| `dot` | DNS over TLS (DoT) | 853 | `1.1.1.1:853` |
| `doh` | DNS over HTTPS (DoH) | 443 | `https://doh.pub/dns-query` |

- Only a **single** DNS server is supported (no fallback list).
- For DoT/DoH with a hostname, the hostname is used as TLS SNI.
- For DoH, the full URL must be specified (e.g. `https://doh.pub/dns-query`).
- Omitting `[dns]` entirely uses the system resolver.

## Exported Metrics

| Metric | Type | Description |
|---|---|---|
| `um_up` | Gauge | 1 if the target is reachable, 0 otherwise |
| `um_request_rtt` | Gauge | Round-trip time in milliseconds **(deprecated, use `um_request_rtt_seconds` instead)** |
| `um_request_rtt_seconds` | Histogram | Round-trip time in seconds (exponential buckets: 1ms–65s) |
| `um_ssl_duration` | Gauge | TLS handshake duration in milliseconds |
| `um_tls_cert_expiry_seconds` | Gauge | Unix timestamp when the TLS certificate expires (0 if not applicable) |
| `um_probes_total` | Counter | Total probe attempts, with `status="success"` or `status="failure"` label |
| `um_probe_duration_seconds` | Histogram | Wall-clock duration of each probe cycle in seconds |
| `um_probes_active` | Gauge | Number of probe tasks currently in-flight |
| `um_consecutive_failures` | Gauge | Consecutive probe failures (resets to 0 on success) |
| `um_last_state_change_timestamp_seconds` | Gauge | Unix timestamp of the last up/down state transition |
| `um_last_success_timestamp_seconds` | Gauge | Unix timestamp of the last successful probe |
| `um_config_reloads_total` | Counter | Total number of successful configuration reloads |
| `um_dns_lookups_total` | Counter | DNS lookups, labeled by `status` (`success`/`failure`), `target`, and `protocol` |
| `um_build_info` | Gauge | Build information (value always 1, labels: `version`, `commit`) |
| `um_http_redirects_total` | Counter | Total number of HTTP redirects followed |
| `um_response_size_bytes` | Gauge | HTTP response body size in bytes |

All metrics carry `target`, `ip`, `protocol`, `port`, and any user-defined `extra_labels`.
