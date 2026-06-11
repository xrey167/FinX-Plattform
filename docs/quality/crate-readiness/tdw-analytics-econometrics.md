# tdw-analytics-econometrics Readiness Worksheet

Generated during the G015 "WS6b Quant and Econometrics" landing (gap-matrix item
**L4.3**), which introduced the pure-Rust regression + econometric-tests crate.

## Evidence Snapshot

- Manifest: `crates/tdw-analytics-econometrics/Cargo.toml`.
- Targets: lib.
- Local deps: `tdw-domain`, plus `schemars`, `serde`, `serde_json`, `thiserror`.
- Reverse deps: `tdw-endpoint-catalog` (the `econometrics/*` Compute routes derive
  their params/model schemas from this crate's typed structs) and
  `tdw-service-api` (the `Op::FetchData` / `Op::ToolCall` compute path).
- Features: none.
- Tests: 23 unit tests — golden OLS checks hand-derived from worked examples,
  Cholesky-solver linear-algebra checks, correlation/VIF checks, and Granger /
  cointegration behavioural checks.
- Docs/examples: crate-readiness worksheet plus module-level docs citing each
  estimator's standard definition and the numeric/conditioning caveats.

## Release Assessment

- The crate is a pure, offline, deterministic numeric library: no async, no I/O,
  no policy. Estimators consume caller-supplied series / design matrices and
  return typed coefficient tables and test statistics.
- Estimators implemented (5 routes): `ols` (coefficients, std errors, t-stats,
  R², adjusted R², F-statistic, Durbin-Watson, residual dof);
  `correlation_matrix` (Pearson); `vif` (variance inflation factors via the
  auxiliary-regression R²); `granger_causality` (the restricted-vs-unrestricted
  F-test at a lag); `cointegration` (Engle-Granger step one plus a residual
  stationarity score).
- Clean-room: every formula is textbook math cited to its standard definition in
  the owning module's docs (Gauss-Markov / normal-equations OLS; Durbin & Watson
  1950; the VIF definition; Granger 1969; Engle & Granger 1987; Dickey-Fuller for
  the residual regression). No reference implementation was consulted; the
  clean-room audit records no exception for this crate.
- The default-feature workspace compiles this crate, so the pedantic/nursery
  ratchet counts it; an enumeration with `-W clippy::pedantic -W clippy::nursery`
  reports zero warnings from its files.

## Numeric approach for OLS

All regression machinery routes through one solver in `linalg.rs`: the normal
equations `(XᵀX) β = Xᵀy` are factored by a hand-rolled Cholesky decomposition
`XᵀX = L Lᵀ` (the Gram matrix is symmetric positive-definite for a full-rank
design) and solved by forward/back substitution. The diagonal of `(XᵀX)⁻¹` —
needed for the coefficient standard errors — is recovered by solving against the
unit vectors. There is **no** third-party linear-algebra dependency (no
`nalgebra` / `faer` / `ndarray`); the workspace pulls none and this crate adds
none.

Conditioning caveat: forming `XᵀX` squares the design's condition number, so the
normal-equations route loses roughly half the available digits versus a direct
QR. For the small, well-scaled designs these routes serve this is more than
adequate, and a rank-deficient design is *detected* rather than silently
mis-solved — the Cholesky factorization fails on a non-positive pivot and the
caller gets `EconometricsError::Singular` instead of garbage coefficients.

## Golden-vector derivation

Expected outputs are hand-derived from the textbook formulas over tiny worked
examples cited in the test comments:

- Exact line `y = 2 + 3x` over `x = [1,2,3,4]` ⇒ intercept 2, slope 3, R² 1.
- Simple-regression worked example `x = [1..5]`, `y = [1,3,2,5,4]`:
  Sxy 8, Sxx 10 ⇒ slope 0.8, intercept 0.6, RSS 3.6, TSS 10 ⇒ R² 0.64, adjusted
  R² 0.52, F 16/3, slope se √0.12 = 0.3464102, slope t 2.3094011.
- Cholesky solve of `A = [[4,2],[2,3]]`, `b = [4,5]` ⇒ `x = [0.25, 1.5]`; Gram of
  a known design matches `[[3,6],[6,14]]`.
- Correlation of perfectly (anti-)linear columns ⇒ ±1; VIF ≥ 1 for orthogonal
  columns; collinear OLS design ⇒ `Singular`.

The `statsmodels` reference values cited in the gap-matrix were *not* used
directly (no Python is available in this clean-room build); the hand-derived
exact-arithmetic figures above are the authoritative expectations.

## Honest simplifications (vs OpenBB)

- **Granger causality** reports the F-statistic and its degrees of freedom but
  not the F-distribution p-value (which needs an incomplete-beta evaluation this
  dependency-free crate omits).
- **Cointegration** implements Engle-Granger step one exactly and scores residual
  stationarity with a named Dickey-Fuller `ρ` slope + t-statistic rather than a
  MacKinnon-table p-value (those critical-value tables are reference data we do
  not embed). The cointegrating regression is exact; the stationarity evidence is
  a well-defined statistic, not a p-value we cannot defensibly compute here.
- **Formal standalone unit-root tests** (ADF / KPSS) are deliberately out of
  scope: ADF needs the same lag-augmentation regression and critical-value tables
  the cointegration module documents away, and KPSS needs a long-run-variance
  estimator. The residual stationarity score inside the cointegration module is
  the one unit-root-flavored statistic shipped.

## Verdict

Ready with follow-ups. Optional OpenBB long-tail econometrics routes
(autocorrelation / residual autocorrelation series, the panel-model family —
random/fixed/between/pooled/first-diff/FMAC — and a formal unit-root route) are
intentionally out of scope for this story and are a later append; the core set
above is complete with numeric tests and is callable as a daemon op
(`econometrics/*` Compute routes) and an MCP tool (`econometrics.*`).
