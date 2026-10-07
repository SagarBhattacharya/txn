# syntax=docker/dockerfile:1

# Stage 1: Build dependency cache & binary
FROM rust:1.99-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency layer separately from application code
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release || true
RUN rm -rf src

# Copy real source code and migrations
COPY src ./src
COPY static ./static
COPY migrations ./migrations

# Invalidate dummy artifacts and build real binary
RUN touch src/main.rs src/lib.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --bin txn

# Stage 2: Minimal runtime image (~60MB)
FROM debian:bookworm-slim AS runner

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled executable
COPY --from=builder /app/target/release/txn /usr/local/bin/txn

# Copy migrations so binary or scripts can apply them
COPY migrations ./migrations

EXPOSE 8000

ENV RUST_LOG="info,txn=debug,tower_http=info"

ENTRYPOINT ["/usr/local/bin/txn"]