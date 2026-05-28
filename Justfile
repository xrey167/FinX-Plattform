set shell := ["powershell.exe", "-NoProfile", "-Command"]

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test-unit:
    cargo test --workspace --no-default-features

test-integration:
    cargo test --workspace --features integration

test-property:
    cargo test --workspace --features property

test-e2e:
    cargo test --workspace --features e2e

# Run the P7 real-backend end-to-end test (requires TDW_POSTGRES_TEST_URL).
#
# Prerequisites:
#   docker compose --profile full up -d
#   export TDW_POSTGRES_TEST_URL="postgres://tdw:tdw@127.0.0.1:5432/tdw"
#
# The test skips cleanly (exit 0) when TDW_POSTGRES_TEST_URL is not set.
test-e2e-pg:
    cargo test --package tdw-service --test g0xx_daemon_e2e_real_pg -- --nocapture

test-adversarial:
    cargo test -p tdw-tag-rules -p tdw-mask -p tdw-auth -p tdw-auth-oidc

coverage:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

coverage-html:
    cargo llvm-cov --workspace --html

bench:
    cargo run -p xtask -- bench

bench-compare baseline:
    cargo run -p xtask -- bench-compare {{baseline}}

mutation crate:
    cargo mutants -p {{crate}}

mutation-core:
    cargo mutants -p tdw-core --features inventory-registration

mutation-changed:
    cargo run -p xtask -- mutation changed

mutation-report out-dir="mutants.out":
    cargo run -p xtask -- mutation report {{out-dir}}

schema-sync:
    cargo run -p xtask -- schema-sync

event-schema-check:
    cargo run -p xtask -- events schema-check

deny:
    cargo deny check

windows-release:
    cargo build --workspace --release --target x86_64-pc-windows-msvc

quality-gate:
    cargo run -p xtask -- quality-gate write

quality-gate-check:
    cargo run -p xtask -- quality-gate check

audit:
    cargo run -p xtask -- clean-room-audit

ci-local:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo test --workspace --no-default-features
    cargo run -p xtask -- schema-sync
    cargo run -p xtask -- events schema-check
    cargo run -p xtask -- quality-gate check
    cargo deny check
    cargo run -p xtask -- clean-room-audit

verify-phase:
    just fmt-check
    just lint
    just test-unit
    just test-integration
    just test-property
    just test-e2e
    just test-adversarial
    just schema-sync
    just event-schema-check
    just bench
    just quality-gate-check
    just deny
    just audit
