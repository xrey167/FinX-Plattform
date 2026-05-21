#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-workflow-engine` crate.

pub const CRATE_NAME: &str = "tdw-workflow-engine";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-workflow-engine");
    }
}
