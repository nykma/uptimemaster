# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Monitoring Domain

### Endpoint
A network target configured for uptime monitoring. Defined by an address (IP, hostname, URL, or MAC), a protocol (tcp, udp, icmp, http, https, arp), and optional checks: expected HTTP status, response body matching, TLS certificate verification, and redirect behavior. Each Endpoint is probed on a configurable interval.

### Probe
A single check cycle against one resolved address of an Endpoint: DNS resolution → network connection → protocol-specific handshake → result measurement. The outcome is captured as a ProbeResult (up/down, round-trip time, TLS certificate expiry, optional HTTP metadata). Probes are rate-limited by a concurrency semaphore.
