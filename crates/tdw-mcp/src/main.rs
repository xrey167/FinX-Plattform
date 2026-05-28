#![forbid(unsafe_code)]

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--stdio-json-rpc") => {
            std::process::exit(tdw_mcp::run_stdio_json_rpc());
        }
        Some("--streamable-http") | Some("--http") => {
            let bind = args
                .next()
                .unwrap_or_else(|| tdw_mcp::default_streamable_http_bind().to_string());
            if args.next().is_some() {
                eprintln!("tdw-mcp Streamable HTTP accepts at most one bind address");
                std::process::exit(2);
            }
            std::process::exit(tdw_mcp::run_streamable_http(&bind));
        }
        Some("--streamable-http-smoke") | Some("--http-smoke") => {
            std::process::exit(tdw_mcp::run_streamable_http_smoke());
        }
        Some(flag) if flag.starts_with("--") => {
            eprintln!("unknown tdw-mcp flag: {flag}");
            std::process::exit(2);
        }
        _ => {}
    }

    match tdw_service_api::mcp_progress_sample("AAPL") {
        Ok(events) => {
            let tools = tdw_service_api::mcp_agent_tools();
            let event = match tdw_service_api::event_spine_sample("mcp") {
                Ok(event) => event,
                Err(error) => {
                    eprintln!("tdw-mcp event error: {error}");
                    std::process::exit(1);
                }
            };
            let kg_tags = match tdw_service_api::kg_tag_sample() {
                Ok(evidence) => evidence,
                Err(error) => {
                    eprintln!("tdw-mcp kg/tag error: {error}");
                    std::process::exit(1);
                }
            };
            let client_events = match tdw_service_api::client_event_sample() {
                Ok(evidence) => evidence,
                Err(error) => {
                    eprintln!("tdw-mcp client event error: {error}");
                    std::process::exit(1);
                }
            };
            println!(
                "tdw-mcp progress={} agent_tools={} tag_tools={} extensibility_tools={} client_events={} event_spine={} kg_tags={}",
                events.join(","),
                tools.join(","),
                tdw_service_api::mcp_tag_tools().join(","),
                tdw_service_api::mcp_extensibility_tools().join(","),
                client_events,
                event,
                kg_tags
            );
        }
        Err(error) => {
            eprintln!("tdw-mcp error: {error}");
            std::process::exit(1);
        }
    }
}
