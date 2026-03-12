# ============================================================
# Stage 1: Chef — Generate dependency recipe for layer caching
# ============================================================
FROM rust:1.76-bookworm AS chef

RUN cargo install cargo-chef --locked
WORKDIR /app

# ============================================================
# Stage 2: Planner — Prepare build recipe from source tree
# ============================================================
FROM chef AS planner

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/

RUN cargo chef prepare --recipe-path recipe.json

# ============================================================
# Stage 3: Builder — Compile release binary (cached deps)
# ============================================================
FROM chef AS builder

# Install build-time dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

# Cook dependencies first (this layer is cached if deps unchanged)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy full source and build
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml deny.toml ./
COPY crates/ crates/

RUN cargo build --release --bin y-agent \
    && strip target/release/y-agent

# ============================================================
# Stage 4: Runtime — Minimal production image
# ============================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd --gid 1001 yagent \
    && useradd --uid 1001 --gid yagent --shell /bin/false --create-home yagent

WORKDIR /app

# Copy binary
COPY --from=builder /app/target/release/y-agent /usr/local/bin/y-agent

# Copy config and migrations
COPY config/ config/
COPY migrations/ migrations/

# Create data directory
RUN mkdir -p /app/data && chown -R yagent:yagent /app

USER yagent

# Default environment
ENV Y_DATA_DIR=/app/data \
    Y_CONFIG_DIR=/app/config \
    RUST_LOG=info

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["y-agent"]
CMD ["serve"]
