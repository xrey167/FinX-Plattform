#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-storage-s3` crate.

pub const CRATE_NAME: &str = "tdw-storage-s3";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-storage-s3");
    }
}
