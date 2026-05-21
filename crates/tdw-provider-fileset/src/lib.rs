#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-provider-fileset` crate.

pub const CRATE_NAME: &str = "tdw-provider-fileset";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-provider-fileset");
    }
}
