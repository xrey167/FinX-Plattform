#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-table-format` crate.

pub const CRATE_NAME: &str = "tdw-table-format";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-table-format");
    }
}
