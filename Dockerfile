# ==========================================
# Stage 1: Build dependency cache & binary
# ==========================================
FROM rust:1.99-slim-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY benches ./benches

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    mkdir -p src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release --bin txn \
    && rm -rf src target/release/deps/txn* target/release/txn*

COPY migrations ./migrations
COPY static ./static
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin txn \
    && cp target/release/txn /tmp/txn

# ==========================================
# Stage 2: Minimal Non-Root Runtime (~40MB)
# ==========================================
FROM debian:bookworm-slim AS runner

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 -s /sbin/nologin appuser

COPY --from=builder /tmp/txn /usr/local/bin/txn

USER appuser

EXPOSE 8000

ENV RUST_LOG="info,txn=debug,tower_http=info"

ENTRYPOINT ["/usr/local/bin/txn"]