//! Tests for the module pipeline: serialization, loading, disassembly, analysis,
//! and formatting. These operate on compiler output and never execute untrusted
//! bytes, so they exercise the "static" half of the crate end to end.

use tessera::analysis;
use tessera::fmt;
use tessera::loader;
use tessera::module::{Const, Module, Proto};
use tessera::serialize;
use tessera::verifier;

fn compile(src: &str) -> Module {
    tessera::compile_source(src, "t").expect("compile failed")
}

#[test]
fn serialize_load_roundtrip_preserves_module() {
    let programs = [
        "return 1;",
        "let x = [1, 2, 3]; return x;",
        "func f(a, b) { return a * b; } return f(6, 7);",
        "func outer(n) { return func() { return n; }; } return outer(5)();",
        "let m = {\"k\": 1}; m[\"j\"] = 2; return len(keys(m));",
    ];
    for src in programs {
        let module = compile(src);
        let bytes = serialize::to_bytes(&module);
        let loaded = loader::load(&bytes).expect("must decode");
        assert_eq!(loaded, module, "roundtrip mismatch for: {src}");
        verifier::verify(&loaded).expect("compiler output must verify");
    }
}

#[test]
fn reserialization_is_stable() {
    let module = compile("func f(x) { if (x > 0) { return x; } return -x; } return f(-3);");
    let bytes1 = serialize::to_bytes(&module);
    let reloaded = loader::load(&bytes1).unwrap();
    let bytes2 = serialize::to_bytes(&reloaded);
    assert_eq!(bytes1, bytes2, "serialization must be deterministic");
}

#[test]
fn disassembly_mentions_key_opcodes() {
    let module = compile("let xs = [1, 2]; return len(xs);");
    let text = tessera::disasm::disassemble(&module);
    assert!(text.contains("newlist"));
    assert!(text.contains("function #0"));
    assert!(text.contains("(entry)"));
}

#[test]
fn analysis_reports_reasonable_metrics() {
    let module = compile(
        "func fib(n) { if (n < 2) { return n; } return fib(n-1) + fib(n-2); } return fib(10);",
    );
    let stats = analysis::module_stats(&module);
    assert!(stats.total_instructions > 5);
    assert!(stats.opcode_histogram.contains_key("call"));
    let complexity = analysis::module_complexity(&module);
    let fib = complexity.iter().find(|c| c.name == "fib").unwrap();
    assert!(fib.calls >= 2);
    assert!(fib.branches >= 1);
}

#[test]
fn unreachable_prototypes_are_detected() {
    // Build a module by hand with a dangling prototype that nothing references.
    let mut m = Module::new();
    let mut orphan = Proto::new("orphan");
    orphan.reg_count = 2;
    orphan.code = vec![
        tessera::bytecode::Instr::LoadNil { dst: 1 },
        tessera::bytecode::Instr::Return { src: 1 },
    ];
    m.add_proto(orphan);

    let mut main = Proto::new("main");
    main.reg_count = 2;
    main.code = vec![
        tessera::bytecode::Instr::LoadInt { dst: 1, imm: 0 },
        tessera::bytecode::Instr::Return { src: 1 },
    ];
    m.entry = m.add_proto(main);

    let unreachable = analysis::unreachable_protos(&m);
    assert_eq!(unreachable, vec![0]);
}

#[test]
fn json_summary_round_trips_names() {
    let module = compile("func greet() { return \"hi\"; } return greet();");
    let json = fmt::module_to_json(&module);
    assert!(json.contains("\"greet\""));
    assert!(json.contains("\"instructions\""));
    assert!(json.contains("\"regs\""));
}

#[test]
fn pretty_printer_output_recompiles() {
    let src = "
        func classify(n) {
            if (n < 0) { return \"neg\"; }
            else if (n == 0) { return \"zero\"; }
            else { return \"pos\"; }
        }
        let r = classify(7);
        return r;
    ";
    let program = tessera::parser::parse(tessera::lexer::tokenize(src).unwrap()).unwrap();
    let printed = fmt::pretty_program(&program);
    // The pretty-printed source must compile back to a verifiable module.
    let recompiled = tessera::compile_source(&printed, "reformatted").unwrap();
    verifier::verify(&recompiled).unwrap();
}

#[test]
fn constant_pool_dedups_repeated_literals() {
    let module = compile("return \"x\" ++ \"x\" ++ \"x\";");
    let count = module
        .consts
        .iter()
        .filter(|c| matches!(c, Const::Str(s) if s == "x"))
        .count();
    assert_eq!(count, 1, "identical string constants should be interned once");
}

#[test]
fn version_mismatch_is_rejected() {
    let module = compile("return 1;");
    let mut bytes = serialize::to_bytes(&module);
    // Bump the version field (offset 4..6) beyond what this build supports.
    bytes[4] = 0xff;
    bytes[5] = 0xff;
    assert!(loader::load(&bytes).is_err());
}

#[test]
fn empty_program_compiles_and_runs() {
    let (value, _out) = tessera::run_source("").expect("empty program should run");
    assert!(value.is_nil());
}
