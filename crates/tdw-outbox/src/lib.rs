#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-outbox` crate.

pub const CRATE_NAME: &str = "tdw-outbox";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-outbox");
    }
}
