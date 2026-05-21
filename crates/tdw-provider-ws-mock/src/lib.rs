#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-provider-ws-mock` crate.

pub const CRATE_NAME: &str = "tdw-provider-ws-mock";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-provider-ws-mock");
    }
}
