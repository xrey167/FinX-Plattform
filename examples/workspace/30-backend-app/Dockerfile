# Build and run example 30 (30-backend-app).
#
# Builds only this example's bin against the workspace's pinned toolchain, then
# ships a slim image that runs it. The example is offline (it boots the in-memory
# daemon / offline stub model in-process).
FROM rust:1.95-slim-bookworm AS build
WORKDIR /workspace
RUN apt-get update     && apt-get install -y --no-install-recommends pkg-config ca-certificates     && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p tdw-workspace-examples --bin ws-30-backend-app --target-dir target

FROM debian:bookworm-slim
RUN apt-get update     && apt-get install -y --no-install-recommends ca-certificates     && rm -rf /var/lib/apt/lists/*     && useradd --create-home --shell /usr/sbin/nologin tdw
COPY --from=build /workspace/target/release/ws-30-backend-app /usr/local/bin/example
USER tdw
ENTRYPOINT ["/usr/local/bin/example"]
