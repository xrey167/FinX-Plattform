#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-eval-runner` crate.

pub const CRATE_NAME: &str = "tdw-eval-runner";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-eval-runner");
    }
}
