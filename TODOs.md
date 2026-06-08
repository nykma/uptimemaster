# TODOs

## ✅ Done

- [x] TLS 证书过期监控 (`um_tls_cert_expiry_seconds`)
- [x] Prometheus Counter (`um_probes_total`)
- [x] HTTP 响应体匹配 (`expected_body` / `expected_body_regex`)
- [x] DNS 降级 + resolve_hostname 超时
- [x] UDP/ARP 阻塞 I/O → async
- [x] HTTP 探测使用 resolve_to_addrs
- [x] 配置热加载连线 + 优雅关闭 + /health

---

## 🟡 第二梯队：锦上添花

- [ ] **自监控指标（Exporter 监控自己）**
  - `um_probe_duration_seconds` — 探测本身耗时（排查调度延迟）
  - `um_probes_active` — 当前飞行中的探测数（排查并发瓶颈）
  - `um_config_reloads_total` — 配置重载次数
  - `um_dns_lookups_total` — DNS 查询次数/成功率
  - 运维 Prometheus 自身需要知道 exporter 是否健康

- [ ] **TLS 版本 / 密码套件信息**
  - `um_tls_version_info{version="1.3"}` — 跟踪哪些服务还在用老 TLS
  - 合规场景很需要

- [ ] **HTTP 重定向追踪**
  ```toml
  follow_redirects = true         # 默认 false
  max_redirects = 5
  ```
  - `um_http_redirects_total` — 重定向次数
  - 有些服务 HTTP→HTTPS 跳转，当前只能测跳转本身而不是最终服务

- [ ] **最近成功时间戳**
  - `um_last_success_timestamp_seconds`
  - `time() - um_last_success_timestamp_seconds > 3600` → "超过 1 小时不通"
  - 比"当前是否在线"更灵活的告警方式

---

## 🔵 第三梯队：可选增强

- [ ] **DNS 记录监控（新 probe 类型 `dns`）**
  ```toml
  [[endpoint]]
  target = "example.com"
  protocol = "dns"
  record_type = "A"               # A | AAAA | MX | CNAME | TXT
  expected_value = "1.2.3.4"      # 可选，校验 DNS 返回内容
  ```
  - DNS 服务本身也是基础设施

- [ ] **响应体大小**
  - `um_response_size_bytes` — HTTP 响应体大小
  - 能捕获 CDN 返回空页面 / 截断响应等问题

- [ ] **端点描述字段**
  ```toml
  description = "生产环境 API 网关"
  ```
  - 纯对人友好，不作为 label，出现在 `/metrics` 的 HELP 注释中

- [ ] **探测间隔 Jitter**
  ```toml
  interval_jitter = 0.1  # ±10% 随机抖动
  ```
  - 避免大量端点同时探测，造成瞬时网络/CPU 尖峰

- [ ] **自定义 User-Agent**
  ```toml
  user_agent = "uptimemaster/0.1.0"
  ```
  - 有些 WAF/CDN 会屏蔽默认的 reqwest UA

---

## ❌ 不做

| 想法 | 原因 |
|---|---|
| Webhook/告警通知 | 不是 Prometheus 的事，交给 Alertmanager |
| Web Dashboard | Grafana 已经做得很好 |
| InfluxDB/Graphite 等后端 | 只对接 Prometheus |
| gRPC 探测 | 复杂度高、受众小 |
| 多实例 HA | Prometheus HA + 多 exporter 实例即可 |
