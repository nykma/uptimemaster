# uptimemaster

用 Rust 编写的网络探活监控守护进程。定期探测你配置的网络端点，并将结果以 [Prometheus](https://prometheus.io/) 指标格式对外暴露。

## 快速开始（Docker Compose）

```bash
mkdir config
cp config.sample.toml config/config.toml
# 根据你的需要编辑 config/config.toml
docker compose up -d
```

所有配置项在 [`config.sample.toml`](config.sample.toml) 中有完整说明。复制为 `config/config.toml`、调整你要监控的端点后即可运行。

Prometheus 指标地址为 `http://localhost:9191/metrics`（端口可在 `[general]` 中修改）。

> **提示：** 容器镜像通过 Nix flake 构建并发布到 `ghcr.io`。详细镜像标签和挂载配置请查看 [`docker-compose.yml`](docker-compose.yml)。
>
> 配置通过**目录**加载（默认 `/config`）。目录下所有 `.toml` 文件按文件名排序后合并读取，详见[配置文件](#配置文件)。

## 支持的探测类型

| 协议 | 说明 | 目标示例 |
|---|---|---|
| `tcp` | TCP 端口连通性探测 | `192.168.1.1:80` |
| `udp` | UDP 数据报文发送/接收 | `8.8.8.8:53` |
| `icmp` | ICMP Ping 探测 | `8.8.8.8` 或 `example.com` |
| `http` | HTTP GET/POST，状态码校验 | `http://example.com/api` |
| `https` | HTTPS 探测，含 TLS 握手耗时 | `https://example.com/health` |
| `arp` | ARP 请求（二层，MAC 地址） | `aa:bb:cc:dd:ee:ff` |

HTTP 探测支持 `GET` / `POST` 两种方法，可配置自定义请求头、JSON 请求体以及期望状态码校验。HTTPS 探测还会额外测量 TLS 握手耗时。

## 重要：ICMP 需要 `NET_RAW` 权限

ICMP Ping 使用了 raw socket，在 Linux 上需要 `CAP_NET_RAW` 能力。**只要任意端点配置了 `protocol = "icmp"`，就必须授予此权限。**

### Docker Compose 中

项目自带的 [`docker-compose.yml`](docker-compose.yml) 已包含：

```yaml
cap_add:
  - NET_RAW
```

直接使用自带 Compose 文件则无需额外操作。

### 直接运行（不使用 Docker）

请以 `root` 身份运行，或为二进制文件赋予能力：

```bash
sudo setcap cap_net_raw+ep ./uptimemaster
```

若缺少 `CAP_NET_RAW`，uptimemaster 将在启动时输出警告，且 ICMP 探针运行时会因权限不足而失败。

### systemd 部署

项目提供了 [systemd unit 模板](contrib/uptimemaster.service)，用于将 uptimemaster 作为系统服务运行：

```bash
# 安装二进制文件
sudo cp uptimemaster /usr/local/bin/uptimemaster
sudo setcap cap_net_raw+ep /usr/local/bin/uptimemaster

# 安装配置
sudo mkdir -p /etc/uptimemaster/config
sudo cp config/*.toml /etc/uptimemaster/config/

# 安装并启用服务
sudo cp contrib/uptimemaster.service /etc/systemd/system/
sudo mkdir -p /var/lib/uptimemaster
sudo systemctl daemon-reload
sudo systemctl enable --now uptimemaster
```

每个 semver 标签推送后，预编译二进制文件（`uptimemaster-x86_64`、`uptimemaster-aarch64`）会发布到 [GitHub Releases](https://github.com/nykma/uptimemaster/releases)。

## 配置文件

uptimemaster 从一个配置目录中加载所有 `.toml` 文件（默认路径 `/config`）。

- **`config.toml`** 最先被处理，且是**唯一**允许定义 `[general]` 和 `[dns]` 的文件。其他文件只能包含 `[[endpoint]]` 条目。
- 其他 `.toml` 文件（如 `servers.toml`、`web.toml`）按文件名排序后合并其端点。
- 热加载：目录中任意 `.toml` 文件的变更都会触发配置重载。

完整配置项及示例见 [`config.sample.toml`](config.sample.toml)，这里快速概览：

```toml
# config/config.toml — 唯一可以写 [general] 和 [dns] 的文件
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

# config/servers.toml — 只放 endpoint
[[endpoint]]
target = "8.8.4.4"
protocol = "icmp"

# config/web.toml — 只放 endpoint
[[endpoint]]
target = "https://example.com/health"
protocol = "https"
method = "get"
expected_status = [200]
expected_body = "ok"  # 可选：校验响应体内容

# config/api.toml — 正则匹配响应体
[[endpoint]]
target = "https://api.example.com/status"
protocol = "https"
method = "get"
expected_status = [200]
expected_body_regex = '^\{"status":"healthy".*\}$'  # 可选：正则匹配响应体
```

- `[general]` — 全局默认值（指标端口、并发数、探测间隔、超时时间）。**仅允许写在 `config.toml`。**
- `[dns]` — 自定义 DNS 解析器配置（可选）。**仅允许写在 `config.toml`。**
- `[[endpoint]]` — 每个端点一个配置块。必填项：`target`、`protocol`，其余字段均有合理默认值。

### DNS 配置（`[dns]`）

默认情况下，uptimemaster 使用系统 DNS 解析器。你可以通过自定义 DNS 服务器来覆盖：

```toml
# config/config.toml
[dns]
server = "1.1.1.1"       # IP 或主机名，可选 :port
protocol = "udp"          # udp | tcp | dot | doh
```

| 协议 | 说明 | 默认端口 | `server` 示例 |
|---|---|---|---|
| `udp` | 标准 UDP DNS | 53 | `1.1.1.1` 或 `8.8.8.8:53` |
| `tcp` | TCP DNS | 53 | `8.8.8.8:53` |
| `dot` | DNS over TLS (DoT) | 853 | `1.1.1.1:853` |
| `doh` | DNS over HTTPS (DoH) | 443 | `https://doh.pub/dns-query` |

- `server` 指定单个 DNS 服务器，`servers` 指定多个服务器（顺序故障转移），两者互斥。
- DoT/DoH 使用主机名时，主机名将作为 TLS SNI。
- DoH 必须填写完整 URL（如 `https://doh.pub/dns-query`）。
- 不配置 `[dns]` 则使用系统解析器。

## 导出的指标

| 指标名 | 类型 | 说明 |
|---|---|---|
| `um_up` | Gauge | 目标可达则为 1，否则为 0 |
| `um_request_rtt` | Gauge | 往返耗时（毫秒）**（已废弃，请使用 `um_request_rtt_seconds` 替代）** |
| `um_request_rtt_seconds` | Histogram | 往返耗时（秒，指数分布桶：1ms–65s） |
| `um_ssl_duration` | Gauge | TLS 握手耗时（毫秒） |
| `um_tls_cert_expiry_seconds` | Gauge | TLS 证书过期 Unix 时间戳（不适用则为 0） |
| `um_probes_total` | Counter | 探测总次数，带 `status="success"` 或 `status="failure"` 标签 |
| `um_probe_duration_seconds` | Histogram | 每次探测周期的墙上时钟耗时（秒） |
| `um_probes_active` | Gauge | 当前正在进行的探测任务数 |
| `um_consecutive_failures` | Gauge | 连续探测失败次数（成功后归零） |
| `um_last_state_change_timestamp_seconds` | Gauge | 最后一次 up/down 状态翻转的 Unix 时间戳 |
| `um_last_success_timestamp_seconds` | Gauge | 最后一次成功探测的 Unix 时间戳 |
| `um_config_reloads_total` | Counter | 配置热加载成功总次数 |
| `um_dns_lookups_total` | Counter | DNS 查询次数，按 `status`（`success`/`failure`）、`target`、`protocol` 区分 |
| `um_build_info` | Gauge | 构建信息（值恒为 1，标签：`version`、`commit`） |
| `um_http_redirects_total` | Counter | HTTP 重定向跟随总次数 |
| `um_response_size_bytes` | Gauge | HTTP 响应体字节数 |

所有指标均带有 `target`、`ip`、`protocol`、`port` 以及用户自定义的 `extra_labels`。
