#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-spatial` crate.

pub const CRATE_NAME: &str = "tdw-spatial";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-spatial");
    }
}
