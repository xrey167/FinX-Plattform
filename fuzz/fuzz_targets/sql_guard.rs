#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_sql_guard` shim (TEST-POLICY-004): drives the
// read-only SQL guard parsing and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_exec::__fuzz_sql_guard(data);
});
