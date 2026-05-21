#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-storage-qdrant` crate.

pub const CRATE_NAME: &str = "tdw-storage-qdrant";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-storage-qdrant");
    }
}
