#![forbid(unsafe_code)]

//! Bootstrap stub for the `tdw-storage-meilisearch` crate.

pub const CRATE_NAME: &str = "tdw-storage-meilisearch";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(CRATE_NAME, "tdw-storage-meilisearch");
    }
}
