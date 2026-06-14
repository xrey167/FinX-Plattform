#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust, offline, deterministic option-pricing math (ecosystem item
//! **G001**).
//!
//! Hand-rolled implementations of the standard textbook option-pricing models
//! over caller-supplied contract inputs. The crate performs no I/O and no async
//! work and uses no global/thread RNG: it is a deterministic numeric library a
//! daemon compute route or a UDF can call directly. Every figure is reproducible
//! from its inputs (the Monte-Carlo pricer takes an explicit `seed`).
//!
//! # Models
//!
//! - [`black_scholes`] — the Black-Scholes-Merton closed form for European call
//!   and put prices and the analytic greeks (delta, gamma, theta, vega, rho),
//!   with a continuous dividend yield `q`.
//! - [`implied_vol`] — inverts a market price for its Black-Scholes implied
//!   volatility via Newton-Raphson with a bracketed bisection fallback (robust,
//!   bounded, converges or returns a clear error).
//! - [`binomial`] — the Cox-Ross-Rubinstein binomial tree for European *and*
//!   American exercise with a continuous dividend yield and a configurable step
//!   count; the European price converges to Black-Scholes as the step count
//!   grows.
//! - [`monte_carlo`] — a seeded geometric-Brownian-motion Monte-Carlo European
//!   pricer with antithetic variates and a reported sampling standard error.
//!
//! # Conventions
//!
//! Rates (`r`) and dividend yield (`q`) are continuously compounded annual
//! decimals; `volatility` (`sigma`) is annualized; `time_to_expiry` (`T`) is in
//! years. See [`params`] for the per-model request and output structs (all
//! `serde` + `JsonSchema` so the catalog can publish their schemas).
//!
//! ```
//! use tdw_quant_options::black_scholes;
//! use tdw_quant_options::params::{BlackScholesParams, OptionType};
//! let call = black_scholes::price(BlackScholesParams {
//!     spot: 100.0,
//!     strike: 100.0,
//!     rate: 0.05,
//!     dividend_yield: 0.0,
//!     volatility: 0.20,
//!     time_to_expiry: 1.0,
//!     option_type: OptionType::Call,
//! })
//! .expect("valid contract");
//! assert!((call - 10.4506).abs() < 1e-3);
//! ```
//!
//! # Clean-room provenance
//!
//! Every formula here is public textbook math cited to its standard definition in
//! the owning module's docs (Black & Scholes 1973; Merton 1973 for the
//! continuous-dividend extension; Cox, Ross & Rubinstein 1979 for the binomial
//! tree; Boyle 1977 for the Monte-Carlo approach; Abramowitz & Stegun 7.1.26 for
//! the error-function approximation behind the normal CDF). No reference
//! implementation was consulted.

pub mod binomial;
pub mod black_scholes;
pub mod error;
pub mod implied_vol;
pub mod monte_carlo;
pub mod normal;
pub mod params;

pub use error::OptionError;

/// Result type for option-pricing computations.
pub type Result<T> = std::result::Result<T, OptionError>;
