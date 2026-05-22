#![forbid(unsafe_code)]

fn main() {
    match tdw_service_api::fetch_equity_historical("fileset", "AAPL") {
        Ok(object) => {
            println!(
                "tdw-cli provider={} endpoint={} rows={}",
                object.provider,
                object.endpoint,
                object.rows.len()
            );
            match tdw_service_api::client_event_sample() {
                Ok(events) => println!("tdw-cli events={}", events["tui_lines"]),
                Err(error) => eprintln!("tdw-cli event sample error: {error}"),
            }
        }
        Err(error) => {
            eprintln!("tdw-cli error: {error}");
            std::process::exit(1);
        }
    }
}
