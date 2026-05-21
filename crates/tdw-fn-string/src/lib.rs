#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-fn-string` crate.

pub const CRATE_NAME: &str = "tdw-fn-string";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-fn-string");
    }
}
