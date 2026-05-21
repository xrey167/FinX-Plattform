#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-embed-google` crate.

pub const CRATE_NAME: &str = "tdw-embed-google";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-embed-google");
    }
}
