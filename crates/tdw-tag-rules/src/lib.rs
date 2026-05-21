#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-tag-rules` crate.

pub const CRATE_NAME: &str = "tdw-tag-rules";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-tag-rules");
    }
}
