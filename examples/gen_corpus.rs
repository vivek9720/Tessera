//! Corpus and proof-of-concept generator.
//!
//! Run once from the repository root:
//!
//! ```text
//! cargo run --example gen_corpus
//! ```
//!
//! It (1) compiles every `*.tsr` seed under `fuzz/corpus/source_fuzzer/` into a
//! serialized module under `fuzz/corpus/module_fuzzer/`, giving the module
//! harness realistic, structure-valid starting inputs, and (2) writes two
//! standalone `.tmod` modules under `pocs/` that reach the two memory-safety
//! faults the module harness targets. Both PoC modules pass the loader and the
//! verifier; the fault occurs only once the VM executes them.

use std::fs;
use std::path::Path;

use tessera::bytecode::Instr;
use tessera::module::{Module, Proto, UpvalDesc};
use tessera::serialize::to_bytes;

fn main() {
    let src_dir = Path::new("fuzz/corpus/source_fuzzer");
    let mod_dir = Path::new("fuzz/corpus/module_fuzzer");
    let poc_dir = Path::new("pocs");
    fs::create_dir_all(mod_dir).expect("create module corpus dir");
    fs::create_dir_all(poc_dir).expect("create pocs dir");

    // 1. Compile source seeds into module seeds.
    let mut compiled = 0usize;
    if let Ok(entries) = fs::read_dir(src_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("tsr") {
                continue;
            }
            let src = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("skip {}: {e}", path.display());
                    continue;
                }
            };
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("seed");
            match tessera::compile_to_bytes(&src, stem) {
                Ok(bytes) => {
                    // Sanity-check that what we emit round-trips and runs.
                    if let Err(e) = tessera::load_module(&bytes) {
                        eprintln!("warning: {stem} did not verify: {e}");
                    }
                    let out = mod_dir.join(format!("seed_{stem}.tmod"));
                    fs::write(&out, &bytes).expect("write module seed");
                    compiled += 1;
                }
                Err(e) => eprintln!("skip {stem}: compile error: {e}"),
            }
        }
    }
    println!("compiled {compiled} source seed(s) into {}", mod_dir.display());

    // 2. Proof-of-concept modules.
    let uaf = poc_upvalue_uaf();
    fs::write(poc_dir.join("upvalue_uaf.tmod"), &uaf).expect("write uaf poc");
    println!("wrote pocs/upvalue_uaf.tmod ({} bytes)", uaf.len());

    let oob = poc_makelist_oob();
    fs::write(poc_dir.join("makelist_oob.tmod"), &oob).expect("write oob poc");
    println!("wrote pocs/makelist_oob.tmod ({} bytes)", oob.len());

    // Confirm both PoCs pass loader + verifier (they should; the fault is at
    // run time, not decode time).
    for (name, bytes) in [("upvalue_uaf", &uaf), ("makelist_oob", &oob)] {
        match tessera::load_module(bytes) {
            Ok(_) => println!("  {name}: loads and verifies (fault is at execution)"),
            Err(e) => eprintln!("  {name}: unexpectedly rejected before execution: {e}"),
        }
    }
}

/// A module that reaches a use-after-free through an escaped closure whose
/// upvalue borrows the top register of an already-returned frame.
///
/// `make` stores a value in its highest register, wraps it in a closure, and
/// returns the closure. The entry function then invokes the returned closure,
/// which reads that upvalue after `make`'s frame — and its register buffer —
/// have been released.
fn poc_upvalue_uaf() -> Vec<u8> {
    let mut m = Module::new();

    let mut inner = Proto::new("inner");
    inner.arity = 0;
    inner.reg_count = 2;
    inner.upvals = vec![UpvalDesc::FromLocal(2)];
    inner.code = vec![
        Instr::GetUpval { dst: 1, uv: 0 },
        Instr::Return { src: 1 },
    ];
    let inner_idx = m.add_proto(inner);

    let mut make = Proto::new("make");
    make.arity = 0;
    make.reg_count = 3;
    make.code = vec![
        Instr::LoadInt { dst: 2, imm: 123 },
        Instr::Closure { dst: 1, proto: inner_idx },
        Instr::Return { src: 1 },
    ];
    let make_idx = m.add_proto(make);

    let mut main = Proto::new("main");
    main.arity = 0;
    main.reg_count = 4;
    main.code = vec![
        Instr::Closure { dst: 1, proto: make_idx },
        Instr::Call { dst: 2, callee: 1, base: 2, argc: 0 },
        Instr::Call { dst: 3, callee: 2, base: 3, argc: 0 },
        Instr::Return { src: 3 },
    ];
    m.entry = m.add_proto(main);

    to_bytes(&m)
}

/// A module whose `MakeList` gathers a register window that extends past the
/// frame's register file, producing an out-of-bounds read.
fn poc_makelist_oob() -> Vec<u8> {
    let mut m = Module::new();
    let mut main = Proto::new("main");
    main.arity = 0;
    main.reg_count = 3;
    main.code = vec![
        Instr::LoadInt { dst: 1, imm: 1 },
        Instr::LoadInt { dst: 2, imm: 2 },
        Instr::MakeList { dst: 1, base: 1, count: 64 },
        Instr::Return { src: 1 },
    ];
    m.entry = m.add_proto(main);
    to_bytes(&m)
}
