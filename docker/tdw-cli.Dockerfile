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
RUN cargo build --release --bin tdw-cli

FROM debian:bookworm-slim AS runtime

# OCI metadata: links the GHCR package to the repo (source), and carries
# the description/license into the registry UI.
LABEL org.opencontainers.image.source="https://github.com/xrey167/FinX-Plattform"       org.opencontainers.image.description="TDW trading-data-warehouse CLI"       org.opencontainers.image.licenses="MIT OR Apache-2.0"
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin tdw
COPY --from=builder /app/target/release/tdw-cli /usr/local/bin/tdw-cli
USER tdw
WORKDIR /home/tdw
ENTRYPOINT ["/usr/local/bin/tdw-cli"]
CMD ["AAPL"]
