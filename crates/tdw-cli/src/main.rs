#![forbid(unsafe_code)]

fn main() {
    match tdw_service_api::fetch_equity_historical("fileset", "AAPL") {
        Ok(object) => println!(
            "tdw-cli provider={} endpoint={} rows={}",
            object.provider,
            object.endpoint,
            object.rows.len()
        ),
        Err(error) => {
            eprintln!("tdw-cli error: {error}");
            std::process::exit(1);
        }
    }
}
