#![no_main]
//! Fuzzing harness for the source path.
//!
//! Interprets the input as UTF-8 Tessera source and runs it through the lexer,
//! parser, compiler, verifier, and virtual machine. This exercises a different
//! entry point than the module harness: the whole front end plus execution of
//! freshly compiled code.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tessera::run_source_bytes(data);
});
