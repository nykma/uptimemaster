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

## 配置文件

uptimemaster 从一个配置目录中加载所有 `.toml` 文件（默认路径 `/config`）。

- **`config.toml`** 最先被处理，且是**唯一**允许定义 `[general]` 的文件。其他文件只能包含 `[[endpoint]]` 条目。
- 其他 `.toml` 文件（如 `dns.toml`、`web.toml`）按文件名排序后合并其端点。
- 热加载：目录中任意 `.toml` 文件的变更都会触发配置重载。

完整配置项及示例见 [`config.sample.toml`](config.sample.toml)，这里快速概览：

```toml
# config/config.toml — 唯一可以写 [general] 的文件
[general]
port = 9191
max_concurrent_probes = 50
default_interval_secs = 30
default_timeout_ms = 5000
extra_labels = { node = "my_home" }

[[endpoint]]
target = "192.168.1.1:80"
protocol = "tcp"

# config/dns.toml — 只放 endpoint
[[endpoint]]
target = "8.8.4.4"
protocol = "icmp"

# config/web.toml — 只放 endpoint
[[endpoint]]
target = "https://example.com/health"
protocol = "https"
method = "get"
expected_status = [200]
```

- `[general]` — 全局默认值（指标端口、并发数、探测间隔、超时时间）。**仅允许写在 `config.toml`。**
- `[[endpoint]]` — 每个端点一个配置块。必填项：`target`、`protocol`，其余字段均有合理默认值。

## 导出的指标

| 指标名 | 类型 | 说明 |
|---|---|---|
| `um_up` | Gauge | 目标可达则为 1，否则为 0 |
| `um_request_rtt_seconds` | Gauge | 往返耗时（秒） |
| `um_ssl_duration_seconds` | Gauge | TLS 握手耗时（仅 HTTPS） |

所有指标均带有 `target`、`ip`、`protocol`、`port` 以及用户自定义的 `extra_labels`。
