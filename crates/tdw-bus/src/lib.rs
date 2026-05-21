#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-bus` crate.

pub const CRATE_NAME: &str = "tdw-bus";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-bus");
    }
}
