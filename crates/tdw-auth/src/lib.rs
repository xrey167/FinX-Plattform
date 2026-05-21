#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-auth` crate.

pub const CRATE_NAME: &str = "tdw-auth";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-auth");
    }
}
