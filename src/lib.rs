//! # Tessera
//!
//! Tessera is a small, embeddable, dynamically-typed scripting language. Source
//! text is compiled to a compact binary *module* — a header, a constant pool,
//! and a table of function prototypes — which a register-based virtual machine
//! loads, verifies, and executes.
//!
//! The crate is organized as a conventional language pipeline:
//!
//! ```text
//! source ──▶ [lexer] ──▶ tokens ──▶ [parser] ──▶ AST ──▶ [compiler] ──▶ Module
//!                                                                          │
//!                                                            [serialize]   │
//!                                                                 ▼        ▼
//!                                          bytes ◀──────────── Module   [vm] ──▶ Value
//!                                            │                              ▲
//!                                            └──▶ [loader] ──▶ [verifier] ──┘
//! ```
//!
//! Embedders typically use one of the convenience entry points:
//! [`compile_source`], [`compile_to_bytes`], [`load_module`], [`run_source`],
//! or [`run_module_bytes`].
//!
//! ## Trust model
//!
//! Modules may come from untrusted sources, so the loader tolerates arbitrary
//! bytes and the verifier proves every operand in range before the VM runs. The
//! VM in turn reports runtime faults as [`Error`] values rather than aborting.

pub mod analysis;
pub mod ast;
pub mod builtins;
pub mod bytecode;
pub mod compiler;
pub mod disasm;
pub mod error;
pub mod fmt;
pub mod heap;
pub mod inspect;
pub mod lexer;
pub mod limits;
pub mod loader;
pub mod manifest;
pub mod module;
pub mod optimize;
pub mod parser;
pub mod reader;
pub mod serialize;
pub mod source_map;
pub mod stdlib;
pub mod text;
pub mod token;
pub mod value;
pub mod verifier;
pub mod vm;
pub mod writer;

pub use error::{Error, ErrorKind, Result};
pub use inspect::{inspect_module, inspect_source, ModuleInspection};
pub use manifest::{parse_manifest, ProjectManifest};
pub use module::Module;
pub use value::Value;
pub use vm::Vm;

/// Magic bytes at the start of every serialized module.
pub const MAGIC: [u8; 4] = *b"TSRA";

/// The current binary module format version. The loader accepts any version in
/// `1..=MODULE_VERSION`.
pub const MODULE_VERSION: u16 = 4;

/// Compile source text into an in-memory [`Module`].
pub fn compile_source(src: &str, name: &str) -> Result<Module> {
    let tokens = lexer::tokenize(src)?;
    let program = parser::parse(tokens)?;
    compiler::compile(&program, name)
}

/// Compile source text with constant folding enabled.
///
/// Produces a module semantically identical to [`compile_source`] but with
/// compile-time-constant subexpressions pre-evaluated. See [`optimize`].
pub fn compile_source_optimized(src: &str, name: &str) -> Result<Module> {
    let tokens = lexer::tokenize(src)?;
    let program = parser::parse(tokens)?;
    let folded = optimize::fold_program(&program);
    compiler::compile(&folded, name)
}

/// Compile source text and serialize it to the binary module format.
pub fn compile_to_bytes(src: &str, name: &str) -> Result<Vec<u8>> {
    let module = compile_source(src, name)?;
    Ok(serialize::to_bytes(&module))
}

/// Decode a module from bytes and run the semantic verifier on it.
pub fn load_module(bytes: &[u8]) -> Result<Module> {
    let module = loader::load(bytes)?;
    verifier::verify(&module)?;
    Ok(module)
}

/// Verify (if not already) and execute a module, returning its result value and
/// captured output.
pub fn eval_module(module: &Module) -> Result<(Value, String)> {
    verifier::verify(module)?;
    let mut vm = Vm::new();
    let value = vm.run_module(module)?;
    Ok((value, vm.take_output()))
}

/// Compile and run source text, returning the program's result value and the
/// text it printed.
pub fn run_source(src: &str) -> Result<(Value, String)> {
    let module = compile_source(src, "main")?;
    eval_module(&module)
}

/// Load a serialized module from raw bytes, verify it, and execute it.
///
/// This is the primary embedding entry point for running precompiled modules,
/// and the one exercised most heavily by the module fuzzing harness: it drives
/// the full decode → verify → execute path over untrusted input.
pub fn run_module_bytes(bytes: &[u8]) -> Result<Value> {
    let module = loader::load(bytes)?;
    verifier::verify(&module)?;
    let mut vm = Vm::new();
    vm.run_module(&module)
}

/// Interpret `bytes` as UTF-8 source and compile + run it. Non-UTF-8 input is a
/// clean error. Used by the source fuzzing harness.
pub fn run_source_bytes(bytes: &[u8]) -> Result<(Value, String)> {
    let src = std::str::from_utf8(bytes)
        .map_err(|_| Error::new(ErrorKind::Lex, "source is not valid UTF-8"))?;
    run_source(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end_arithmetic() {
        let (v, _out) = run_source("return 6 * 7;").unwrap();
        assert_eq!(v.as_int(), Some(42));
    }

    #[test]
    fn module_roundtrip_runs() {
        let bytes = compile_to_bytes("let x = 10; return x + 5;", "m").unwrap();
        let v = run_module_bytes(&bytes).unwrap();
        assert_eq!(v.as_int(), Some(15));
    }

    #[test]
    fn print_is_captured() {
        let (_v, out) = run_source(r#"println("hello"); return 0;"#).unwrap();
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn closures_work_end_to_end() {
        let src = "func make(n) { return func() { return n * n; }; } let f = make(9); return f();";
        let (v, _out) = run_source(src).unwrap();
        assert_eq!(v.as_int(), Some(81));
    }

    #[test]
    fn garbage_bytes_do_not_panic() {
        // A grab-bag of malformed inputs must all resolve to Err, never panic.
        for seed in 0u16..2000 {
            let bytes = seed.to_le_bytes();
            let _ = run_module_bytes(&bytes);
            let _ = run_source_bytes(&bytes);
        }
    }
}
