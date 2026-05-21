#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-udf-js` crate.

pub const CRATE_NAME: &str = "tdw-udf-js";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-udf-js");
    }
}
