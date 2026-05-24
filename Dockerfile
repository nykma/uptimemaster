# Stage 1: Build
FROM rust:1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libpcap-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY src ./src
RUN touch src/main.rs
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libpcap0.8 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/uptimemaster /usr/local/bin/uptimemaster

ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

EXPOSE 9191

ENTRYPOINT ["/usr/local/bin/uptimemaster"]
CMD ["-c", "/config"]
