#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-embed` crate.

pub const CRATE_NAME: &str = "tdw-embed";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-embed");
    }
}
