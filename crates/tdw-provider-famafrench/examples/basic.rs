//! Offline example for `tdw-provider-famafrench`.
//!
//! Mirrors the fetcher path without network: builds a query with
//! `transform_query`, then parses an inline Ken French factor CSV table with the
//! offline `parse_factor_table` parser. The published table is percent-valued;
//! the parser converts each value to a decimal fraction.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-famafrench --example basic --features http
//! ```

use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_famafrench::{FamaFrenchHttpFetcher, parse_factor_table};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = FamaFrenchHttpFetcher::transform_query(json!({
        "factor_set": "3factor",
        "frequency": "daily"
    }))?;
    println!(
        "resolved dataset: {} -> {}",
        query.dataset().zip_file,
        query.dataset().csv_member
    );

    let csv = "\
This file was created by CMPT_ME_BEME_RETS using the 202404 CRSP database.

,Mkt-RF,SMB,HML,RF
20240603,0.55,-0.21,0.13,0.022
20240604,-0.34,0.10,-0.05,0.022
";
    let rows = parse_factor_table(csv)?;
    println!("decoded {} factor row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} mkt_rf={:?} smb={:?} hml={:?} rf={:?}",
            row.date, row.mkt_rf, row.smb, row.hml, row.rf
        );
    }
    Ok(())
}
