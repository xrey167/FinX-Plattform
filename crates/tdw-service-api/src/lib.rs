#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-service-api` crate.

pub const CRATE_NAME: &str = "tdw-service-api";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-service-api");
    }
}
