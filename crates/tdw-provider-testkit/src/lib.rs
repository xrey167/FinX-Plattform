//! Dev-only test helpers shared across the `tdw-provider-*` HTTP fetcher
//! integration tests.
//!
//! These macros collapse two byte-identical boilerplate block families that
//! recur across `crates/tdw-provider-*/tests/http_fetcher.rs`:
//!
//! * [`cassette_bytes!`] — the cassette builder tail
//!   `Bytes::from(json!(...).to_string().into_bytes())`.
//! * [`live_fetch_nonempty!`] — the live-fetch `extract_data` / `transform_data`
//!   chain whose `unwrap_or_else(|e| panic!("live ... must succeed: {e}"))`
//!   panic strings are byte-identical at every site.
//!
//! The crate is intentionally dev-only: it depends only on `bytes`,
//! `serde_json`, and `tdw-core` (for [`tdw_core::Credentials`]). It performs no
//! network access and pulls in no `reqwest`.

/// Build a [`bytes::Bytes`] cassette body from a `serde_json` literal.
///
/// Expands to `Bytes::from(json!(<literal>).to_string().into_bytes())`,
/// preserving the JSON fixture contents exactly. The literal is forwarded as a
/// token tree so embedded commas in arrays/objects pass through unchanged.
#[macro_export]
macro_rules! cassette_bytes {
    ($($json:tt)*) => {
        ::bytes::Bytes::from(::serde_json::json!($($json)*).to_string().into_bytes())
    };
}

/// Run the standard live-fetch `extract_data` -> `transform_data` chain and
/// return the decoded rows vec.
///
/// Expands to the byte-identical body
///
/// ```ignore
/// {
///     let raw = $fetcher
///         .extract_data(&$query, &Credentials::default())
///         .await
///         .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
///     $fetcher
///         .transform_data(&$query, raw)
///         .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"))
/// }
/// ```
///
/// The expansion evaluates to the rows vec so the caller can bind it
/// (`let rows = live_fetch_nonempty!(fetcher, query);`) and keep its own
/// per-provider trailing assertions (`assert!(!rows.is_empty(), ...)`,
/// `assert_eq!(rows[0].series_id, ...)`) byte-for-byte. The panic strings match
/// the existing call sites exactly after expansion.
#[macro_export]
macro_rules! live_fetch_nonempty {
    ($fetcher:expr, $query:expr) => {{
        let raw = $fetcher
            .extract_data(&$query, &::tdw_core::Credentials::default())
            .await
            .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
        $fetcher
            .transform_data(&$query, raw)
            .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"))
    }};
}
