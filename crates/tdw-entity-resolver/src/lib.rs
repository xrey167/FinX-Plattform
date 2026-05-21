#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-entity-resolver` crate.

pub const CRATE_NAME: &str = "tdw-entity-resolver";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-entity-resolver");
    }
}
