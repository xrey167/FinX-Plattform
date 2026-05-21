#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-embed-openai` crate.

pub const CRATE_NAME: &str = "tdw-embed-openai";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-embed-openai");
    }
}
