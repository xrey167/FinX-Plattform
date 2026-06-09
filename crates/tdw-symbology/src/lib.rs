//! Ticker-symbology normalization utilities.
//!
//! This crate provides mapping and normalization functions over dot-suffix ticker symbols
//! (e.g. `AAPL`, `RY.TO`, `SHEL.L`):
//!
//! - [`exchange_label`] — maps a ticker's dot-suffix to a human-readable
//!   exchange name (e.g. `".TO"` → `"Toronto Stock Exchange"`).  Returns
//!   `None` for unknown or absent suffixes.
//!
//! - [`vendor_symbol`] — converts a dot-suffix ticker to the charting-vendor
//!   `"EXCHANGE:TICKER"` form (e.g. `"RY.TO"` → `"TSX:RY"`).  Suffix
//!   matching uses the **longest suffix first** so that multi-char suffixes
//!   like `".SS"` are preferred over a hypothetical single-char `".S"`.
//!   Inputs are normalised (trim + uppercase) before matching.  Tickers with
//!   no suffix, or with a suffix not in the table, are returned uppercase and
//!   bare (US-style, no prefix), e.g. `"aapl"` → `"AAPL"`.
//!
//! - [`normalize_symbol`] — canonical trim + ASCII-uppercase normalization
//!   shared by alerting/watchlist storage.

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
/// Maps a ticker dot-suffix to a human-readable exchange label.
///
/// The suffix must include the leading dot and be upper-case (the caller is
/// responsible for normalisation; see [`vendor_symbol`] for an example of
/// how to extract and normalise the suffix before calling this function).
///
/// Returns `None` when the suffix is not in the whitelist.
///
/// # Examples
///
/// ```
/// assert_eq!(tdw_symbology::exchange_label(".TO"), Some("Toronto Stock Exchange"));
/// assert_eq!(tdw_symbology::exchange_label(".ZZ"), None);
/// ```
#[must_use]
pub fn exchange_label(suffix: &str) -> Option<&'static str> {
    // ~45-entry whitelist ordered alphabetically by suffix for readability.
    // The match arm strings are intentionally `&'static str` so callers can
    // store them without allocation.
    match suffix {
        ".AS" => Some("Euronext Amsterdam"),
        ".AT" => Some("Athens Stock Exchange"),
        ".AX" => Some("Australian Securities Exchange"),
        ".BA" => Some("Buenos Aires Stock Exchange"),
        ".BD" => Some("Budapest Stock Exchange"),
        ".BK" => Some("Stock Exchange of Thailand"),
        ".BO" => Some("BSE India"),
        ".BR" => Some("Euronext Brussels"),
        ".CN" => Some("Canadian Securities Exchange"),
        ".CO" => Some("Nasdaq Copenhagen"),
        ".DE" => Some("XETRA"),
        ".HE" => Some("Nasdaq Helsinki"),
        ".HK" => Some("Hong Kong Stock Exchange"),
        ".IC" => Some("Nasdaq Iceland"),
        ".IS" => Some("Borsa Istanbul"),
        ".JK" => Some("Indonesia Stock Exchange"),
        ".JO" => Some("Johannesburg Stock Exchange"),
        ".KL" => Some("Bursa Malaysia"),
        ".KQ" => Some("KOSDAQ"),
        ".KS" => Some("Korea Stock Exchange"),
        ".L" => Some("London Stock Exchange"),
        ".LS" => Some("Euronext Lisbon"),
        ".MC" => Some("Bolsa de Madrid"),
        ".MI" => Some("Borsa Italiana"),
        ".MX" => Some("Mexican Stock Exchange"),
        ".NE" => Some("NEO Exchange"),
        ".NS" => Some("NSE India"),
        ".NZ" => Some("New Zealand Exchange"),
        ".OL" => Some("Oslo Stock Exchange"),
        ".PA" => Some("Euronext Paris"),
        ".PR" => Some("Prague Stock Exchange"),
        ".SA" => Some("B3 Brazil"),
        ".SI" => Some("Singapore Exchange"),
        ".SN" => Some("Santiago Stock Exchange"),
        ".SS" => Some("Shanghai Stock Exchange"),
        ".ST" => Some("Nasdaq Stockholm"),
        ".SW" => Some("SIX Swiss Exchange"),
        ".SZ" => Some("Shenzhen Stock Exchange"),
        ".T" => Some("Tokyo Stock Exchange"),
        ".TA" => Some("Tel Aviv Stock Exchange"),
        ".TO" => Some("Toronto Stock Exchange"),
        ".TW" => Some("Taiwan Stock Exchange"),
        ".V" => Some("TSX Venture Exchange"),
        ".VI" => Some("Vienna Stock Exchange"),
        ".WA" => Some("Warsaw Stock Exchange"),
        _ => None,
    }
}

/// Converts a dot-suffix ticker to the charting-vendor `"EXCHANGE:TICKER"` form.
///
/// ## Normalisation
/// Input is trimmed and uppercased before any processing.
///
/// ## Suffix matching
/// The function finds the **last** dot in the normalised symbol and treats
/// everything from that dot onward as the suffix.  Suffix lookup uses the
/// longest match: because all suffixes are unambiguous in this table (no
/// suffix is a trailing substring of another), a single last-dot split is
/// sufficient.
///
/// ## Fallback rule
/// If the symbol contains no dot, or the suffix is not in the vendor-code
/// table, the function returns the bare uppercased ticker without a prefix
/// (US-style passthrough), e.g. `"aapl"` → `"AAPL"`.
///
/// # Examples
///
/// ```
/// assert_eq!(tdw_symbology::vendor_symbol("RY.TO"), "TSX:RY");
/// assert_eq!(tdw_symbology::vendor_symbol("aapl"),  "AAPL");
/// assert_eq!(tdw_symbology::vendor_symbol("VOD.L"), "LSE:VOD");
/// ```
#[must_use]
pub fn vendor_symbol(symbol: &str) -> String {
    let normalised = normalize_symbol(symbol);
    if normalised.is_empty() {
        return String::new();
    }

    // Find the last dot to extract suffix.
    if let Some(dot_pos) = normalised.rfind('.') {
        let ticker = &normalised[..dot_pos];
        let suffix = &normalised[dot_pos..];

        if let Some(prefix) = vendor_prefix(suffix) {
            return format!("{prefix}:{ticker}");
        }
    }

    // No dot or unknown suffix → US-style bare passthrough.
    normalised
}

/// Returns the charting-vendor exchange-code prefix for a known dot-suffix,
/// or `None` if the suffix is not in the table.
fn vendor_prefix(suffix: &str) -> Option<&'static str> {
    match suffix {
        ".AS" => Some("AMS"),
        ".AT" => Some("ATSE"),
        ".AX" => Some("ASX"),
        ".BA" => Some("BYMA"),
        ".BD" => Some("BET"),
        ".BK" => Some("SET"),
        ".BO" => Some("BSE"),
        ".BR" => Some("EBR"),
        ".CN" => Some("CNSX"),
        ".CO" => Some("OMXCOP"),
        ".DE" => Some("XETR"),
        ".HE" => Some("OMXHEX"),
        ".HK" => Some("HKEX"),
        ".IC" => Some("OMXICE"),
        ".IS" => Some("BIST"),
        ".JK" => Some("IDX"),
        ".JO" => Some("JSE"),
        ".KL" => Some("KLSE"),
        ".KQ" => Some("KOSDAQ"),
        ".KS" => Some("KRX"),
        ".L" => Some("LSE"),
        ".LS" => Some("EURONEXT"),
        ".MC" => Some("BME"),
        ".MI" => Some("MIL"),
        ".MX" => Some("BMV"),
        ".NE" => Some("NEO"),
        ".NS" => Some("NSE"),
        ".NZ" => Some("NZX"),
        ".OL" => Some("OSL"),
        ".PA" => Some("EPA"),
        ".PR" => Some("PSE"),
        ".SA" => Some("BVMF"),
        ".SI" => Some("SGX"),
        ".SN" => Some("BCS"),
        ".SS" => Some("SSE"),
        ".ST" => Some("STO"),
        ".SW" => Some("SIX"),
        ".SZ" => Some("SZSE"),
        ".T" => Some("TSE"),
        ".TA" => Some("TASE"),
        ".TO" => Some("TSX"),
        ".TW" => Some("TWSE"),
        ".V" => Some("TSXV"),
        ".VI" => Some("WBAG"),
        ".WA" => Some("GPW"),
        _ => None,
    }
}

/// Normalize a raw ticker symbol for consistent storage and comparison.
///
/// The canonical form is the input with leading/trailing whitespace stripped
/// and all ASCII letters converted to uppercase. Non-ASCII characters are
/// preserved as-is (they are unusual in ticker symbols but should not be
/// silently dropped).
///
/// # Examples
///
/// ```
/// use tdw_symbology::normalize_symbol;
///
/// assert_eq!(normalize_symbol("aapl"), "AAPL");
/// assert_eq!(normalize_symbol("  BTC-USD  "), "BTC-USD");
/// assert_eq!(normalize_symbol("ETH/USDT"), "ETH/USDT");
/// ```
#[must_use]
pub fn normalize_symbol(raw: &str) -> String {
    raw.trim().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── exchange_label ────────────────────────────────────────────────────────

    #[test]
    fn exchange_label_known_suffixes() {
        assert_eq!(exchange_label(".TO"), Some("Toronto Stock Exchange"));
        assert_eq!(exchange_label(".L"), Some("London Stock Exchange"));
        assert_eq!(exchange_label(".DE"), Some("XETRA"));
        assert_eq!(exchange_label(".PA"), Some("Euronext Paris"));
        assert_eq!(exchange_label(".HK"), Some("Hong Kong Stock Exchange"));
        assert_eq!(exchange_label(".T"), Some("Tokyo Stock Exchange"));
        assert_eq!(
            exchange_label(".AX"),
            Some("Australian Securities Exchange")
        );
        assert_eq!(exchange_label(".NS"), Some("NSE India"));
        assert_eq!(exchange_label(".BO"), Some("BSE India"));
        assert_eq!(exchange_label(".SW"), Some("SIX Swiss Exchange"));
        assert_eq!(exchange_label(".MI"), Some("Borsa Italiana"));
        assert_eq!(exchange_label(".MC"), Some("Bolsa de Madrid"));
        assert_eq!(exchange_label(".AS"), Some("Euronext Amsterdam"));
        assert_eq!(exchange_label(".BR"), Some("Euronext Brussels"));
        assert_eq!(exchange_label(".SS"), Some("Shanghai Stock Exchange"));
        assert_eq!(exchange_label(".SZ"), Some("Shenzhen Stock Exchange"));
        assert_eq!(exchange_label(".KS"), Some("Korea Stock Exchange"));
        assert_eq!(exchange_label(".KQ"), Some("KOSDAQ"));
        assert_eq!(exchange_label(".TW"), Some("Taiwan Stock Exchange"));
        assert_eq!(exchange_label(".SA"), Some("B3 Brazil"));
        assert_eq!(exchange_label(".SI"), Some("Singapore Exchange"));
        assert_eq!(exchange_label(".BK"), Some("Stock Exchange of Thailand"));
        assert_eq!(exchange_label(".JK"), Some("Indonesia Stock Exchange"));
        assert_eq!(exchange_label(".V"), Some("TSX Venture Exchange"));
        assert_eq!(exchange_label(".CN"), Some("Canadian Securities Exchange"));
        assert_eq!(exchange_label(".NE"), Some("NEO Exchange"));
    }

    #[test]
    fn exchange_label_unknown_suffix_returns_none() {
        assert_eq!(exchange_label(".ZZ"), None);
        assert_eq!(exchange_label(".XX"), None);
        assert_eq!(exchange_label(""), None);
        assert_eq!(exchange_label("TO"), None); // missing dot
    }

    // ── vendor_symbol ─────────────────────────────────────────────────────────

    #[test]
    fn vendor_symbol_known_suffixes_across_regions() {
        // Americas
        assert_eq!(vendor_symbol("RY.TO"), "TSX:RY");
        assert_eq!(vendor_symbol("SHOP.TO"), "TSX:SHOP");
        assert_eq!(vendor_symbol("VALE3.SA"), "BVMF:VALE3");
        assert_eq!(vendor_symbol("GGAL.BA"), "BYMA:GGAL");
        assert_eq!(vendor_symbol("AMXL.MX"), "BMV:AMXL");
        assert_eq!(vendor_symbol("ACB.V"), "TSXV:ACB");
        assert_eq!(vendor_symbol("SU.CN"), "CNSX:SU");
        assert_eq!(vendor_symbol("GFL.NE"), "NEO:GFL");

        // Europe
        assert_eq!(vendor_symbol("VOD.L"), "LSE:VOD");
        assert_eq!(vendor_symbol("ADS.DE"), "XETR:ADS");
        assert_eq!(vendor_symbol("AI.PA"), "EPA:AI");
        assert_eq!(vendor_symbol("ASML.AS"), "AMS:ASML");
        assert_eq!(vendor_symbol("NESN.SW"), "SIX:NESN");
        assert_eq!(vendor_symbol("UCG.MI"), "MIL:UCG");
        assert_eq!(vendor_symbol("SAN.MC"), "BME:SAN");
        assert_eq!(vendor_symbol("ING.BR"), "EBR:ING");
        assert_eq!(vendor_symbol("EDP.LS"), "EURONEXT:EDP");
        assert_eq!(vendor_symbol("VIE.VI"), "WBAG:VIE");
        assert_eq!(vendor_symbol("ERIC-B.ST"), "STO:ERIC-B");
        assert_eq!(vendor_symbol("DNB.OL"), "OSL:DNB");
        assert_eq!(vendor_symbol("NOVO-B.CO"), "OMXCOP:NOVO-B");
        assert_eq!(vendor_symbol("NOKIA.HE"), "OMXHEX:NOKIA");
        assert_eq!(vendor_symbol("MAREL.IC"), "OMXICE:MAREL");
        assert_eq!(vendor_symbol("PKN.WA"), "GPW:PKN");
        assert_eq!(vendor_symbol("CEZ.PR"), "PSE:CEZ");
        assert_eq!(vendor_symbol("OTP.BD"), "BET:OTP");
        assert_eq!(vendor_symbol("TITAN.AT"), "ATSE:TITAN");
        assert_eq!(vendor_symbol("THYAO.IS"), "BIST:THYAO");
        assert_eq!(vendor_symbol("TEVA.TA"), "TASE:TEVA");

        // Asia-Pacific
        assert_eq!(vendor_symbol("7203.T"), "TSE:7203");
        assert_eq!(vendor_symbol("0700.HK"), "HKEX:0700");
        assert_eq!(vendor_symbol("600519.SS"), "SSE:600519");
        assert_eq!(vendor_symbol("000858.SZ"), "SZSE:000858");
        assert_eq!(vendor_symbol("2330.TW"), "TWSE:2330");
        assert_eq!(vendor_symbol("005930.KS"), "KRX:005930");
        assert_eq!(vendor_symbol("247540.KQ"), "KOSDAQ:247540");
        assert_eq!(vendor_symbol("RELIANCE.NS"), "NSE:RELIANCE");
        assert_eq!(vendor_symbol("TCS.BO"), "BSE:TCS");
        assert_eq!(vendor_symbol("CBA.AX"), "ASX:CBA");
        assert_eq!(vendor_symbol("AIR.NZ"), "NZX:AIR");
        assert_eq!(vendor_symbol("D05.SI"), "SGX:D05");
        assert_eq!(vendor_symbol("ADVANC.BK"), "SET:ADVANC");
        assert_eq!(vendor_symbol("BBRI.JK"), "IDX:BBRI");
        assert_eq!(vendor_symbol("KLBF.KL"), "KLSE:KLBF");

        // Africa / LatAm
        assert_eq!(vendor_symbol("SOL.JO"), "JSE:SOL");
        assert_eq!(vendor_symbol("ENTEL.SN"), "BCS:ENTEL");
    }

    #[test]
    fn vendor_symbol_no_suffix_passthrough() {
        assert_eq!(vendor_symbol("AAPL"), "AAPL");
        assert_eq!(vendor_symbol("MSFT"), "MSFT");
        assert_eq!(vendor_symbol("GOOGL"), "GOOGL");
    }

    #[test]
    fn vendor_symbol_unknown_suffix_passthrough() {
        // Unknown suffix → treated as bare ticker (the dot-suffix stays as part
        // of the uppercased passthrough because it is not in the table).
        assert_eq!(vendor_symbol("FOO.ZZ"), "FOO.ZZ");
        assert_eq!(vendor_symbol("BAR.XX"), "BAR.XX");
    }

    #[test]
    fn vendor_symbol_lowercase_normalisation() {
        assert_eq!(vendor_symbol("ry.to"), "TSX:RY");
        assert_eq!(vendor_symbol("vod.l"), "LSE:VOD");
        assert_eq!(vendor_symbol("aapl"), "AAPL");
        assert_eq!(vendor_symbol("  aapl  "), "AAPL");
    }

    #[test]
    fn vendor_symbol_empty_string() {
        assert_eq!(vendor_symbol(""), "");
        assert_eq!(vendor_symbol("   "), "");
    }

    /// Longest-suffix-first precedence: a symbol whose ticker body contains a
    /// dot (e.g. a hypothetical ".S" vs ".SS" clash).  In this codebase the
    /// table has no suffix that is a trailing substring of another, but the
    /// `rfind('.')` strategy naturally prefers the rightmost (longest from the
    /// end) split, which is the intended behaviour.
    ///
    /// Concrete test: "600519.SS" must resolve to `SSE:600519`, not produce
    /// some spurious partial match on ".S" (which is not even in the table,
    /// but the strategy is still correct).
    #[test]
    fn vendor_symbol_longest_suffix_precedence() {
        // ".SS" (Shanghai) is 3 chars; if the implementation only checked the
        // last character it would fail to find the suffix.
        assert_eq!(vendor_symbol("600519.SS"), "SSE:600519");
        // Similarly ".KQ" vs a hypothetical single-char clash.
        assert_eq!(vendor_symbol("247540.KQ"), "KOSDAQ:247540");
        // ".SZ" (Shenzhen) must not be confused with ".S" + "Z".
        assert_eq!(vendor_symbol("000858.SZ"), "SZSE:000858");
    }

    #[test]
    fn lowercase_is_uppercased() {
        assert_eq!(normalize_symbol("aapl"), "AAPL");
    }

    #[test]
    fn already_normalized_is_unchanged() {
        assert_eq!(normalize_symbol("AAPL"), "AAPL");
    }

    #[test]
    fn leading_and_trailing_whitespace_stripped() {
        assert_eq!(normalize_symbol("  btc  "), "BTC");
    }

    #[test]
    fn leading_whitespace_only_stripped() {
        assert_eq!(normalize_symbol("   eth"), "ETH");
    }

    #[test]
    fn trailing_whitespace_only_stripped() {
        assert_eq!(normalize_symbol("eth   "), "ETH");
    }

    #[test]
    fn mixed_case_with_separator() {
        assert_eq!(normalize_symbol("btc-usd"), "BTC-USD");
    }

    #[test]
    fn slash_separator_preserved() {
        assert_eq!(normalize_symbol("eth/usdt"), "ETH/USDT");
    }

    #[test]
    fn empty_string_returns_empty() {
        assert_eq!(normalize_symbol(""), "");
    }

    #[test]
    fn whitespace_only_returns_empty() {
        assert_eq!(normalize_symbol("   "), "");
    }

    #[test]
    fn digits_preserved() {
        assert_eq!(normalize_symbol("sp500"), "SP500");
    }
}
