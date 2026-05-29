#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_mcp_http` shim (TEST-POLICY-004): drives the
// Streamable HTTP request body parsing and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_mcp::__fuzz_mcp_http(data);
});
