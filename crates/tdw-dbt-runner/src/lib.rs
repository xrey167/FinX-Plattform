#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-dbt-runner` crate.

pub const CRATE_NAME: &str = "tdw-dbt-runner";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-dbt-runner");
    }
}
