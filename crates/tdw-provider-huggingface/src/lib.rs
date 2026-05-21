#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-provider-huggingface` crate.

pub const CRATE_NAME: &str = "tdw-provider-huggingface";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-provider-huggingface");
    }
}
