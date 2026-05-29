#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_mcp_jsonrpc` shim (TEST-POLICY-004): drives
// `handle_json_rpc_line` parsing and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_mcp::__fuzz_mcp_jsonrpc(data);
});
