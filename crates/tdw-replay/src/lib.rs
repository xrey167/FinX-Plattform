#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-replay` crate.

pub const CRATE_NAME: &str = "tdw-replay";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-replay");
    }
}
