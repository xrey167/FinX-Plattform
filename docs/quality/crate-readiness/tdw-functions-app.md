# tdw-functions-app Readiness Worksheet

Generated during the OpenStock integration tier (slice B) when the welcome
application function was added on `user.created`.

## Evidence Snapshot

- Manifest: `crates/tdw-functions-app/Cargo.toml`.
- Targets: lib.
- Features: `default` (no I/O); `smtp` enables `TransactionalWelcomeMailer` wrapping `tdw_email::TransactionalMailer`.
- Local deps: tdw-functions, tdw-email.
- Reverse deps: none yet (registered by the running service in a later wiring slice).
- Test attributes found in Rust sources: 5.
- Tests directory: False.
- Docs/examples: crate-readiness worksheet only.
- Scan signal files: 1.
- Related PR coverage: integration slice B adds the `tdw-functions-app` crate hosting the welcome `FunctionDef`, event-triggered on `user.created`, with a `WelcomeMailer` port for offline testability.

## Release Assessment

- Manifest and target shape are visible to `cargo metadata` and represented in `matrix.md`.
- The default build has no network or storage I/O; the live SMTP path is gated behind the `smtp` feature and the presence of `TDW_SMTP_HOST`/`TDW_EMAIL_FROM`.
- The welcome function is clean-room: implemented from the integration-tier functional spec and existing repo patterns. No AGPL code copied. No clean-room exception is recorded for this crate in the current audit pass.
- Any code-level follow-up remains non-blocking unless `fmt`, `clippy -D warnings`, tests, clean-room audit, or `crate-readiness-check` fails.

## Verdict

Ready with follow-ups. The crate ships the welcome function with a tested mailer port; wiring it into the running service is tracked as a separate integration step.
