//! A textual disassembler for compiled modules.
//!
//! The disassembler is used by the `tessera dis` subcommand and by tests that
//! want to assert on generated code. It renders each prototype's header (arity,
//! register count, upvalue descriptors) followed by its instruction stream with
//! resolved constant previews.

use crate::bytecode::Instr;
use crate::module::{Const, Module, Proto, UpvalDesc};
use std::fmt::Write as _;

/// Render an entire module as human-readable assembly.
pub fn disassemble(module: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "; module {:?}  version={}  consts={}  protos={}  entry=#{}",
        module.name,
        module.version,
        module.consts.len(),
        module.protos.len(),
        module.entry
    );
    if !module.consts.is_empty() {
        out.push_str("; constant pool\n");
        for (i, c) in module.consts.iter().enumerate() {
            let _ = writeln!(out, ";   [{i}] {}", render_const(c));
        }
    }
    for (i, proto) in module.protos.iter().enumerate() {
        disassemble_proto(&mut out, module, i, proto);
    }
    out
}

fn render_const(c: &Const) -> String {
    match c {
        Const::Int(v) => format!("int {v}"),
        Const::Float(v) => format!("float {v}"),
        Const::Str(s) => format!("str {s:?}"),
        Const::Bytes(b) => format!("bytes <{} bytes>", b.len()),
    }
}

fn disassemble_proto(out: &mut String, module: &Module, idx: usize, proto: &Proto) {
    let marker = if idx as u32 == module.entry { " (entry)" } else { "" };
    let _ = writeln!(
        out,
        "\nfunction #{idx} {:?}{marker}  arity={} regs={} upvals={}",
        proto.name,
        proto.arity,
        proto.reg_count,
        proto.upvals.len()
    );
    for (i, uv) in proto.upvals.iter().enumerate() {
        let desc = match uv {
            UpvalDesc::FromLocal(reg) => format!("local r{reg}"),
            UpvalDesc::FromUpval(idx) => format!("parent upvalue {idx}"),
        };
        let _ = writeln!(out, "    ; upvalue {i} <- {desc}");
    }
    for (pc, instr) in proto.code.iter().enumerate() {
        let _ = writeln!(out, "  {pc:>4}  {}", render_instr(module, *instr));
    }
}

fn render_instr(module: &Module, instr: Instr) -> String {
    let m = instr.mnemonic();
    match instr {
        Instr::LoadNil { dst } | Instr::LoadTrue { dst } | Instr::LoadFalse { dst } => {
            format!("{m:<10} r{dst}")
        }
        Instr::LoadConst { dst, k } => {
            format!("{m:<10} r{dst}, k{k}    ; {}", const_preview(module, k))
        }
        Instr::LoadInt { dst, imm } => format!("{m:<10} r{dst}, {imm}"),
        Instr::Move { dst, src } => format!("{m:<10} r{dst}, r{src}"),
        Instr::Add { dst, a, b }
        | Instr::Sub { dst, a, b }
        | Instr::Mul { dst, a, b }
        | Instr::Div { dst, a, b }
        | Instr::Mod { dst, a, b }
        | Instr::Pow { dst, a, b }
        | Instr::Eq { dst, a, b }
        | Instr::Ne { dst, a, b }
        | Instr::Lt { dst, a, b }
        | Instr::Le { dst, a, b }
        | Instr::Gt { dst, a, b }
        | Instr::Ge { dst, a, b }
        | Instr::Concat { dst, a, b } => format!("{m:<10} r{dst}, r{a}, r{b}"),
        Instr::Neg { dst, a } | Instr::Not { dst, a } | Instr::TypeOf { dst, a } => {
            format!("{m:<10} r{dst}, r{a}")
        }
        Instr::Jump { target } => format!("{m:<10} -> {target}"),
        Instr::JumpIfFalse { cond, target } | Instr::JumpIfTrue { cond, target } => {
            format!("{m:<10} r{cond}, -> {target}")
        }
        Instr::Call { dst, callee, base, argc } => {
            format!("{m:<10} r{dst}, r{callee}, base r{base}, argc {argc}")
        }
        Instr::TailCall { callee, base, argc } => {
            format!("{m:<10} r{callee}, base r{base}, argc {argc}")
        }
        Instr::Return { src } => format!("{m:<10} r{src}"),
        Instr::MakeList { dst, base, count } | Instr::MakeMap { dst, base, count } => {
            format!("{m:<10} r{dst}, base r{base}, count {count}")
        }
        Instr::Index { dst, obj, key } => format!("{m:<10} r{dst}, r{obj}[r{key}]"),
        Instr::SetIndex { obj, key, val } => format!("{m:<10} r{obj}[r{key}] = r{val}"),
        Instr::Len { dst, obj } => format!("{m:<10} r{dst}, r{obj}"),
        Instr::Append { obj, val } => format!("{m:<10} r{obj}, r{val}"),
        Instr::GetGlobal { dst, name } => {
            format!("{m:<10} r{dst}, g{name}    ; {}", const_preview(module, name))
        }
        Instr::SetGlobal { name, src } => {
            format!("{m:<10} g{name}, r{src}    ; {}", const_preview(module, name))
        }
        Instr::GetUpval { dst, uv } => format!("{m:<10} r{dst}, u{uv}"),
        Instr::SetUpval { uv, src } => format!("{m:<10} u{uv}, r{src}"),
        Instr::Closure { dst, proto } => format!("{m:<10} r{dst}, #{proto}"),
        Instr::Nop | Instr::Halt => m.to_string(),
    }
}

fn const_preview(module: &Module, k: u32) -> String {
    match module.consts.get(k as usize) {
        Some(c) => render_const(c),
        None => "<invalid>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disassembles_a_compiled_module() {
        let program = crate::parser::parse(crate::lexer::tokenize("return 1 + 2;").unwrap()).unwrap();
        let module = crate::compiler::compile(&program, "t").unwrap();
        let text = disassemble(&module);
        assert!(text.contains("function #0"));
        assert!(text.contains("add"));
    }
}
