#![no_main]
//! Fuzzing harness for the binary module path.
//!
//! Feeds raw bytes to `tessera::run_module_bytes`, exercising the full
//! decode → verify → execute pipeline over untrusted module bytes: the loader,
//! the bytecode verifier, and the register virtual machine (including closures,
//! upvalues, and collection construction). This is the harness that drives the
//! stateful interpreter, so it reaches the deepest, most lifecycle-sensitive
//! code in the crate.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The result is intentionally discarded: well-formed and malformed inputs
    // alike should either run to completion or return an `Err`, never panic or
    // corrupt memory. A sanitizer abort is what we are hunting for.
    let _ = tessera::run_module_bytes(data);
});
