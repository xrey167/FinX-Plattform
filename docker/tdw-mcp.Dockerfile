ARG RUST_VERSION=1.95.0
ARG CARGO_CHEF_VERSION=0.1.77

FROM rust:${RUST_VERSION}-bookworm AS chef
ARG CARGO_CHEF_VERSION
RUN cargo install cargo-chef --version "${CARGO_CHEF_VERSION}" --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin tdw-mcp

FROM debian:bookworm-slim AS runtime
# `curl` is included so the compose/K8s healthcheck can probe the MCP ops
# endpoints (/health, /ready) on TDW_MCP_OPS_BIND.
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin tdw
COPY --from=builder /app/target/release/tdw-mcp /usr/local/bin/tdw-mcp
USER tdw
WORKDIR /home/tdw
ENTRYPOINT ["/usr/local/bin/tdw-mcp"]
