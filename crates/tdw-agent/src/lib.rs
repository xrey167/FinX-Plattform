#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-agent` crate.

pub const CRATE_NAME: &str = "tdw-agent";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-agent");
    }
}
