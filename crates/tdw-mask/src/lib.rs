#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-mask` crate.

pub const CRATE_NAME: &str = "tdw-mask";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-mask");
    }
}
