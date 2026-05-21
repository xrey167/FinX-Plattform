#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-feature-store` crate.

pub const CRATE_NAME: &str = "tdw-feature-store";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-feature-store");
    }
}
