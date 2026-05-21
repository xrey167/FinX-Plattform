#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-auth-oidc` crate.

pub const CRATE_NAME: &str = "tdw-auth-oidc";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-auth-oidc");
    }
}
