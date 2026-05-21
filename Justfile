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

coverage:
    cargo llvm-cov nextest --workspace --lcov --output-path lcov.info

coverage-html:
    cargo llvm-cov nextest --workspace --html

bench:
    cargo run -p xtask -- bench

bench-compare baseline:
    cargo run -p xtask -- bench-compare {{baseline}}

mutation crate:
    cargo mutants -p {{crate}}

audit:
    cargo run -p xtask -- clean-room-audit

ci-local:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo run -p xtask -- clean-room-audit
