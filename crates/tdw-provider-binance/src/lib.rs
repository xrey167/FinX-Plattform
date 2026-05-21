#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-provider-binance` crate.

pub const CRATE_NAME: &str = "tdw-provider-binance";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-provider-binance");
    }
}
