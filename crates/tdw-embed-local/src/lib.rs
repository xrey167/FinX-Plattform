#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-embed-local` crate.

pub const CRATE_NAME: &str = "tdw-embed-local";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-embed-local");
    }
}
