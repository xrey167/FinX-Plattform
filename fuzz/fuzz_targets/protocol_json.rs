#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_protocol_json` shim (TEST-POLICY-004): drives
// `OpEnvelope`/`EventMsg`/`ReplayFrame` JSON parsing and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_protocol::__fuzz_protocol_json(data);
});
