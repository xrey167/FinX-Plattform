#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-agent-store` crate.

pub const CRATE_NAME: &str = "tdw-agent-store";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-agent-store");
    }
}
