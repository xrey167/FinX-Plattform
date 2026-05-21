#![forbid(unsafe_code)]

fn main() {
    match tdw_service_api::fetch_equity_historical("yahoo", "MSFT") {
        Ok(object) => match tdw_service_api::event_spine_sample("worker") {
            Ok(event) => println!(
                "tdw-worker job=equity_historical provider={} rows={} event_spine={}",
                object.provider,
                object.rows.len(),
                event
            ),
            Err(error) => {
                eprintln!("tdw-worker event error: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("tdw-worker error: {error}");
            std::process::exit(1);
        }
    }
}
