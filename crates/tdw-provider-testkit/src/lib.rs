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

#![deny(clippy::pedantic, clippy::nursery)]
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

/// Run the live-fetch `extract_data` -> `transform_data` chain using the
/// `.expect()` panic mechanism and return the decoded rows vec.
///
/// Expands to the byte-identical body
///
/// ```ignore
/// {
///     let raw = $fetcher
///         .extract_data(&$query, &Credentials::default())
///         .await
///         .expect("live extract_data must succeed");
///     $fetcher
///         .transform_data(&$query, raw)
///         .expect("live transform_data must succeed")
/// }
/// ```
///
/// This is the sibling of [`live_fetch_nonempty!`] for the call sites that use
/// `.expect("live ... must succeed")` rather than
/// `unwrap_or_else(|e| panic!(...))`. The two panic mechanisms are kept as
/// distinct macros so no panic string changes. The expansion evaluates to the
/// rows vec so the caller can bind it
/// (`let rows = live_fetch_rows_expect!(fetcher, query);`) and keep its own
/// per-provider trailing assertions byte-for-byte. Dev-only, no network.
#[macro_export]
macro_rules! live_fetch_rows_expect {
    ($fetcher:expr, $query:expr) => {{
        let raw = $fetcher
            .extract_data(&$query, &::tdw_core::Credentials::default())
            .await
            .expect("live extract_data must succeed");
        $fetcher
            .transform_data(&$query, raw)
            .expect("live transform_data must succeed")
    }};
}

/// Run a live `fetch` and assert the returned object's `rows` are non-empty.
///
/// Expands to the byte-identical body
///
/// ```ignore
/// {
///     let obbject = $fetcher
///         .fetch($value, &Credentials::default())
///         .await
///         .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));
///     assert!(!obbject.rows.is_empty(), $msg);
/// }
/// ```
///
/// The `fetch` argument (`$value`) and the non-emptiness assertion message
/// (`$msg`) are forwarded verbatim so each provider keeps its exact literal.
/// The `"live fetch must succeed: {e}"` panic string is byte-identical at every
/// call site. Dev-only, no network.
#[macro_export]
macro_rules! live_fetch_obbject_nonempty {
    ($fetcher:expr, $value:expr, $msg:expr) => {{
        let obbject = $fetcher
            .fetch($value, &::tdw_core::Credentials::default())
            .await
            .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));
        assert!(!obbject.rows.is_empty(), $msg);
    }};
}
