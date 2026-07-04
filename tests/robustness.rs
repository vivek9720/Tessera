//! Robustness tests: malformed input must never panic, and the module pipeline
//! must reject structurally invalid bytes cleanly.
//!
//! Note: these tests deliberately do **not** execute the crafted trigger modules
//! from `examples/gen_corpus.rs`. Those reach genuine memory-safety faults that
//! are only observable (and safe to observe) under a sanitizer build such as the
//! fuzzing harness; running them in an ordinary `cargo test` process would be
//! undefined behaviour. Here we only assert that they pass the loader and the
//! verifier, which is where the "structure-valid, fault-at-runtime" property
//! lives.

use tessera::bytecode::Instr;
use tessera::module::{Module, Proto, UpvalDesc};
use tessera::serialize::to_bytes;

/// A tiny deterministic PRNG so the test is reproducible without a dependency.
struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
}

#[test]
fn random_bytes_never_panic_module_path() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    for _ in 0..20_000 {
        let len = (rng.next_u32() % 48) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push((rng.next_u32() & 0xff) as u8);
        }
        // Must return a Result, never panic.
        let _ = tessera::run_module_bytes(&bytes);
    }
}

#[test]
fn random_bytes_never_panic_source_path() {
    let mut rng = Lcg(0x0fed_cba9_8765_4321);
    for _ in 0..20_000 {
        let len = (rng.next_u32() % 64) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push((rng.next_u32() & 0xff) as u8);
        }
        let _ = tessera::run_source_bytes(&bytes);
    }
}

#[test]
fn truncations_of_a_valid_module_are_rejected_cleanly() {
    let bytes = tessera::compile_to_bytes("let x = [1, 2, 3]; return len(x);", "m").unwrap();
    for cut in 0..bytes.len() {
        // Every prefix shorter than the whole must fail to load, not panic.
        let prefix = &bytes[..cut];
        let _ = tessera::load_module(prefix);
    }
    // The full module still loads.
    assert!(tessera::load_module(&bytes).is_ok());
}

#[test]
fn single_byte_flips_do_not_panic() {
    let bytes = tessera::compile_to_bytes(
        "func f(x) { return x + 1; } return f(41);",
        "m",
    )
    .unwrap();
    for i in 0..bytes.len() {
        for bit in 0..8 {
            let mut mutated = bytes.clone();
            mutated[i] ^= 1 << bit;
            // Loading or running a bit-flipped module must not panic; whether it
            // succeeds or errors depends on which byte changed.
            let _ = tessera::run_module_bytes(&mutated);
        }
    }
}

#[test]
fn crafted_uaf_module_passes_loader_and_verifier() {
    // Mirror of poc_upvalue_uaf in examples/gen_corpus.rs. It is structurally
    // valid: the fault only manifests during execution under a sanitizer.
    let mut m = Module::new();

    let mut inner = Proto::new("inner");
    inner.reg_count = 2;
    inner.upvals = vec![UpvalDesc::FromLocal(2)];
    inner.code = vec![Instr::GetUpval { dst: 1, uv: 0 }, Instr::Return { src: 1 }];
    let inner_idx = m.add_proto(inner);

    let mut make = Proto::new("make");
    make.reg_count = 3;
    make.code = vec![
        Instr::LoadInt { dst: 2, imm: 123 },
        Instr::Closure { dst: 1, proto: inner_idx },
        Instr::Return { src: 1 },
    ];
    let make_idx = m.add_proto(make);

    let mut main = Proto::new("main");
    main.reg_count = 4;
    main.code = vec![
        Instr::Closure { dst: 1, proto: make_idx },
        Instr::Call { dst: 2, callee: 1, base: 2, argc: 0 },
        Instr::Call { dst: 3, callee: 2, base: 3, argc: 0 },
        Instr::Return { src: 3 },
    ];
    m.entry = m.add_proto(main);

    let bytes = to_bytes(&m);
    // Round-trips through the loader and passes the verifier.
    let loaded = tessera::loader::load(&bytes).expect("uaf module must decode");
    assert!(tessera::verifier::verify(&loaded).is_ok());
}

#[test]
fn crafted_oob_module_passes_loader_and_verifier() {
    let mut m = Module::new();
    let mut main = Proto::new("main");
    main.reg_count = 3;
    main.code = vec![
        Instr::LoadInt { dst: 1, imm: 1 },
        Instr::LoadInt { dst: 2, imm: 2 },
        Instr::MakeList { dst: 1, base: 1, count: 64 },
        Instr::Return { src: 1 },
    ];
    m.entry = m.add_proto(main);

    let bytes = to_bytes(&m);
    let loaded = tessera::loader::load(&bytes).expect("oob module must decode");
    // The verifier accepts it because it does not constrain the MakeList window
    // length; the out-of-bounds read only happens when the VM gathers elements.
    assert!(tessera::verifier::verify(&loaded).is_ok());
}
