#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-actor` crate.

pub const CRATE_NAME: &str = "tdw-actor";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-actor");
    }
}
