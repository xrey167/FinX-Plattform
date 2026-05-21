#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-provider-polygon` crate.

pub const CRATE_NAME: &str = "tdw-provider-polygon";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-provider-polygon");
    }
}
