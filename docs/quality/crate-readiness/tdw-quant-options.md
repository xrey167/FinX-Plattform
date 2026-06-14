# tdw-quant-options Readiness Worksheet

Generated during the openbb-ecosystem-p1 **G001** landing (the computed
option-pricing module), which introduced the pure-Rust, offline, deterministic
option-pricing crate.

## Evidence Snapshot

- Manifest: `crates/tdw-quant-options/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain`, plus `schemars`, `serde`, `serde_json`, `thiserror`.
  No heavy numeric dependency: the normal CDF is a hand-rolled Abramowitz-Stegun
  erf approximation, and the Monte-Carlo RNG is a hand-rolled seeded SplitMix64,
  so `statrs`/`libm`/`rand` are intentionally NOT dependencies.
- Reverse deps: `tdw-endpoint-catalog` (the `derivatives/pricing/*` Compute
  routes derive their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` / `Op::ToolCall` compute path via
  `options_compute`).
- Features: none.
- Tests: 31 unit tests plus a crate-level doctest — golden-value checks per model
  against textbook fixtures (BS call/put, put-call parity with and without a
  dividend yield, greek signs/bounds and reference values, IV solver
  price->vol->price round-trips, binomial->Black-Scholes convergence, American
  early-exercise premium, seeded-Monte-Carlo determinism and BS tolerance),
  normal-CDF table checks, and param-default tests. The `options_compute` daemon
  wiring adds 9 more tests in `tdw-service-api`.
- Docs/examples: this crate-readiness worksheet plus module-level docs citing
  each model's standard definition, and a runnable crate-level doctest for the
  Black-Scholes price.

## Release Assessment

- The crate is a pure, offline, deterministic numeric library: no async, no I/O,
  no policy, no global/thread RNG. Every figure is reproducible from its inputs;
  the Monte-Carlo pricer takes an explicit `seed`, so its output is reproducible
  too (the repository determinism rule).
- Models implemented (5 routes): `black_scholes` (European call/put price with a
  continuous dividend yield), `greeks` (analytic delta/gamma/theta/vega/rho),
  `implied_volatility` (Newton-Raphson with a bracketed bisection fallback),
  `binomial` (Cox-Ross-Rubinstein tree, European AND American exercise, with a
  continuous dividend yield and a configurable step count), and `monte_carlo`
  (seeded GBM with antithetic variates and a reported sampling standard error).
- Clean-room: every formula is public textbook math cited to its standard
  definition in the owning module's docs (Black & Scholes 1973; Merton 1973 for
  the continuous-dividend extension; Cox, Ross & Rubinstein 1979 for the binomial
  tree; Boyle 1977 for the Monte-Carlo approach; Abramowitz & Stegun 7.1.26 for
  the error-function approximation behind the normal CDF). No reference
  implementation was consulted; the clean-room audit records no exception for
  this crate.
- The default-feature workspace compiles this crate (analytics/pricing are not
  feature-gated), so the pedantic/nursery ratchet counts it; an enumeration with
  `-W clippy::pedantic -W clippy::nursery` reports zero warnings from its files.

## Golden-vector derivation

Expected outputs in the unit tests are the standard textbook figures for the
canonical fixture `S=100, K=100, r=0.05, sigma=0.20, T=1, q=0`:

- Black-Scholes call = 10.4506; put = 5.5735 (put = call - S + K*e^{-rT}).
- Put-call parity `C - P = S*e^{-qT} - K*e^{-rT}` holds to 1e-9, with and
  without a dividend yield.
- Greeks: ATM call/put delta straddle 0 and 1 (call) / -1 and 0 (put), gamma
  ~0.01876 and vega ~37.524 (shared by call and put), ATM theta negative for
  both, call rho > 0 and put rho < 0.
- Implied volatility round-trips: pricing at sigma=0.20 and inverting recovers
  0.20 to better than 1e-6.
- Binomial: a 500-step European tree lands within 0.05 of the Black-Scholes
  value, and is strictly closer than a 10-step tree (convergence); an American
  call with no dividends equals the European call, while an American put carries
  a strictly positive early-exercise premium.
- Monte-Carlo: a fixed seed reproduces the estimate exactly; 200k antithetic
  paths land within 0.1 of the Black-Scholes call.

No external reference values were used directly (this is a clean-room build); the
textbook figures above are the authoritative expectations.

## Verdict

Ready with follow-ups. Optional long-tail extensions (a full IV surface solver,
discrete-dividend schedules, American Monte-Carlo via Longstaff-Schwartz,
exotic-payoff support) are intentionally out of scope for this story and are a
later append; the core set above is complete with numeric tests and is callable
as a daemon op (`derivatives/pricing/*` Compute routes) and an MCP tool
(`derivatives.pricing.*`).
