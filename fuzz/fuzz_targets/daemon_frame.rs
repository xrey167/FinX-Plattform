#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_daemon_frame` shim (TEST-POLICY-004): drives the
// length-delimited daemon event frame reader and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_app_client::__fuzz_daemon_frame(data);
});
