#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-sql-codegen` crate.

pub const CRATE_NAME: &str = "tdw-sql-codegen";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-sql-codegen");
    }
}
