#![forbid(unsafe_code)]

fn main() {
    match tdw_service_api::endpoint_response("fileset", "AAPL") {
        Ok(response) => match (
            tdw_service_api::event_spine_sample("service"),
            tdw_service_api::parity_layer_sample(),
        ) {
            (Ok(event), Ok(parity)) => println!("{response} event_spine={event} parity={parity}"),
            (Err(error), _) | (_, Err(error)) => {
                eprintln!("tdw-service runtime error: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("tdw-service error: {error}");
            std::process::exit(1);
        }
    }
}
