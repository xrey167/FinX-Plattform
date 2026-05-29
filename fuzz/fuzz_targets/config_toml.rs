#![no_main]

use libfuzzer_sys::fuzz_target;

// Reuses the stable `__fuzz_config_toml` shim (TEST-POLICY-004): drives
// `ConfigLayer::from_toml` parsing and asserts no panic.
fuzz_target!(|data: &[u8]| {
    tdw_config::__fuzz_config_toml(data);
});
