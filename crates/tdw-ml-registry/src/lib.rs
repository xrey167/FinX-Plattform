#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-ml-registry` crate.

pub const CRATE_NAME: &str = "tdw-ml-registry";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-ml-registry");
    }
}
