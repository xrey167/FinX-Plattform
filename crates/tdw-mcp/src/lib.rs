#![forbid(unsafe_code)]

use std::io::BufRead;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    id: Option<Value>,
}

#[derive(Debug, Serialize, PartialEq)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, PartialEq)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub fn run_stdio_json_rpc() -> i32 {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => println!("{}", handle_json_rpc_line(&line)),
            Err(error) => {
                eprintln!("tdw-mcp JSON-RPC read error: {error}");
                return 1;
            }
        }
    }
    0
}

pub fn handle_json_rpc_line(line: &str) -> String {
    let response = match serde_json::from_str::<JsonRpcRequest>(line) {
        Ok(request) => handle_json_rpc_request(request),
        Err(_) => error_response(None, -32700, "parse error"),
    };
    match serde_json::to_string(&response) {
        Ok(encoded) => encoded,
        Err(error) => format!(
            r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":"serialize error: {error}"}}}}"#
        ),
    }
}

fn handle_json_rpc_request(request: JsonRpcRequest) -> JsonRpcResponse {
    if request.jsonrpc != "2.0" {
        return error_response(request.id, -32600, "invalid request");
    }

    match request.method.as_str() {
        "ping" => success_response(request.id, json!({ "ok": true })),
        "tools/list" => success_response(
            request.id,
            json!({
                "tools": mcp_tool_catalog(),
                "params": request.params.unwrap_or(Value::Null),
            }),
        ),
        _ => error_response(request.id, -32601, "method not found"),
    }
}

pub fn mcp_tool_catalog() -> Vec<String> {
    let mut tools = tdw_service_api::mcp_agent_tools();
    tools.extend(tdw_service_api::mcp_tag_tools());
    tools.extend(tdw_service_api::mcp_extensibility_tools());
    tools
}

fn success_response(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn error_response(id: Option<Value>, code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_tools_list_returns_catalog() {
        let response = handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let decoded: Value = serde_json::from_str(&response).expect("response json");

        assert_eq!(decoded["jsonrpc"], "2.0");
        assert_eq!(decoded["id"], 1);
        assert!(decoded["result"]["tools"].as_array().expect("tools").len() >= 3);
    }

    #[test]
    fn json_rpc_rejects_malformed_and_unknown_method() {
        let malformed: Value =
            serde_json::from_str(&handle_json_rpc_line("{")).expect("malformed response");
        assert_eq!(malformed["error"]["code"], -32700);

        let unknown: Value = serde_json::from_str(&handle_json_rpc_line(
            r#"{"jsonrpc":"2.0","id":"x","method":"unknown"}"#,
        ))
        .expect("unknown response");
        assert_eq!(unknown["id"], "x");
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
