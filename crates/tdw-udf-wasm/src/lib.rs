#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-udf-wasm` crate.

pub const CRATE_NAME: &str = "tdw-udf-wasm";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-udf-wasm");
    }
}
