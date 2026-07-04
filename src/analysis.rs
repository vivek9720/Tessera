//! Static analysis and metrics over a compiled [`Module`].
//!
//! Nothing here executes code or mutates the module; these are read-only passes
//! used by the `tessera stat` subcommand, by tooling, and by the test suite to
//! sanity-check compiler output. The analyses include an opcode histogram,
//! per-prototype complexity metrics, prototype reachability from the entry
//! point, and a conservative estimate of each function's operand-stack usage.

use std::collections::{BTreeMap, HashSet};

use crate::bytecode::Instr;
use crate::module::{Module, Proto};

/// Aggregate metrics for a whole module.
#[derive(Debug, Clone, Default)]
pub struct ModuleStats {
    pub proto_count: usize,
    pub const_count: usize,
    pub total_instructions: usize,
    pub total_upvalues: usize,
    pub max_registers: u16,
    pub opcode_histogram: BTreeMap<&'static str, usize>,
    pub unreachable_protos: Vec<usize>,
}

impl ModuleStats {
    /// Render the stats as a short human-readable report.
    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "prototypes:        {}", self.proto_count);
        let _ = writeln!(out, "constants:         {}", self.const_count);
        let _ = writeln!(out, "instructions:      {}", self.total_instructions);
        let _ = writeln!(out, "upvalues:          {}", self.total_upvalues);
        let _ = writeln!(out, "max registers:     {}", self.max_registers);
        if !self.unreachable_protos.is_empty() {
            let _ = writeln!(out, "unreachable protos: {:?}", self.unreachable_protos);
        }
        let _ = writeln!(out, "opcode histogram:");
        // Sort by descending frequency, then name for stability.
        let mut rows: Vec<(&&'static str, &usize)> = self.opcode_histogram.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (name, count) in rows {
            let _ = writeln!(out, "  {name:<12} {count}");
        }
        out
    }
}

/// Compute aggregate statistics for a module.
pub fn module_stats(module: &Module) -> ModuleStats {
    let mut histogram: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut total_instructions = 0usize;
    let mut total_upvalues = 0usize;
    let mut max_registers = 0u16;

    for proto in &module.protos {
        total_instructions += proto.code.len();
        total_upvalues += proto.upvals.len();
        max_registers = max_registers.max(proto.reg_count);
        for instr in &proto.code {
            *histogram.entry(instr.mnemonic()).or_insert(0) += 1;
        }
    }

    ModuleStats {
        proto_count: module.protos.len(),
        const_count: module.consts.len(),
        total_instructions,
        total_upvalues,
        max_registers,
        opcode_histogram: histogram,
        unreachable_protos: unreachable_protos(module),
    }
}

/// Per-prototype complexity metrics.
#[derive(Debug, Clone, Default)]
pub struct ProtoComplexity {
    pub name: String,
    pub instructions: usize,
    /// Number of branch instructions (jumps + conditional jumps).
    pub branches: usize,
    /// Number of call sites (including tail calls).
    pub calls: usize,
    /// Cyclomatic-style complexity: branch count plus one.
    pub cyclomatic: usize,
    /// Distinct prototypes instantiated via `closure`.
    pub nested_closures: usize,
}

/// Compute complexity metrics for a single prototype.
pub fn proto_complexity(proto: &Proto) -> ProtoComplexity {
    let mut branches = 0;
    let mut calls = 0;
    let mut nested = 0;
    for instr in &proto.code {
        match instr {
            Instr::Jump { .. } | Instr::JumpIfFalse { .. } | Instr::JumpIfTrue { .. } => {
                branches += 1
            }
            Instr::Call { .. } | Instr::TailCall { .. } => calls += 1,
            Instr::Closure { .. } => nested += 1,
            _ => {}
        }
    }
    ProtoComplexity {
        name: proto.name.clone(),
        instructions: proto.code.len(),
        branches,
        calls,
        cyclomatic: branches + 1,
        nested_closures: nested,
    }
}

/// Complexity metrics for every prototype in a module.
pub fn module_complexity(module: &Module) -> Vec<ProtoComplexity> {
    module.protos.iter().map(proto_complexity).collect()
}

/// Which prototypes are reachable from the entry point, following `closure`
/// instructions transitively.
pub fn reachable_protos(module: &Module) -> HashSet<usize> {
    let mut reachable = HashSet::new();
    let mut stack = Vec::new();
    let entry = module.entry as usize;
    if entry < module.protos.len() {
        stack.push(entry);
    }
    while let Some(idx) = stack.pop() {
        if !reachable.insert(idx) {
            continue;
        }
        if let Some(proto) = module.protos.get(idx) {
            for instr in &proto.code {
                if let Instr::Closure { proto, .. } = instr {
                    let target = *proto as usize;
                    if target < module.protos.len() && !reachable.contains(&target) {
                        stack.push(target);
                    }
                }
            }
        }
    }
    reachable
}

/// Prototype indices that cannot be reached from the entry point.
pub fn unreachable_protos(module: &Module) -> Vec<usize> {
    let reachable = reachable_protos(module);
    (0..module.protos.len())
        .filter(|i| !reachable.contains(i))
        .collect()
}

/// A conservative upper bound on the highest register index a prototype touches.
/// Because the compiler tracks a register high-water mark, this should never
/// exceed `reg_count`; a larger value indicates hand-written or corrupted code.
pub fn max_register_used(proto: &Proto) -> u32 {
    let mut hi = 0u32;
    let mut note = |r: u16| hi = hi.max(r as u32 + 1);
    for instr in &proto.code {
        match *instr {
            Instr::LoadNil { dst }
            | Instr::LoadTrue { dst }
            | Instr::LoadFalse { dst }
            | Instr::LoadConst { dst, .. }
            | Instr::LoadInt { dst, .. } => note(dst),
            Instr::Move { dst, src } => {
                note(dst);
                note(src);
            }
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
            | Instr::Concat { dst, a, b } => {
                note(dst);
                note(a);
                note(b);
            }
            Instr::Neg { dst, a } | Instr::Not { dst, a } | Instr::TypeOf { dst, a } => {
                note(dst);
                note(a);
            }
            Instr::JumpIfFalse { cond, .. } | Instr::JumpIfTrue { cond, .. } => note(cond),
            Instr::Call { dst, callee, base, argc } => {
                note(dst);
                note(callee);
                if argc > 0 {
                    note(base + argc - 1);
                }
            }
            Instr::TailCall { callee, base, argc } => {
                note(callee);
                if argc > 0 {
                    note(base + argc - 1);
                }
            }
            Instr::Return { src } => note(src),
            Instr::MakeList { dst, base, count } | Instr::MakeMap { dst, base, count } => {
                note(dst);
                // Windows may legally reach base + span - 1.
                let span = if matches!(instr, Instr::MakeMap { .. }) {
                    count.saturating_mul(2)
                } else {
                    count
                };
                if span > 0 {
                    note(base.saturating_add(span - 1));
                }
            }
            Instr::Index { dst, obj, key } => {
                note(dst);
                note(obj);
                note(key);
            }
            Instr::SetIndex { obj, key, val } => {
                note(obj);
                note(key);
                note(val);
            }
            Instr::Len { dst, obj } => {
                note(dst);
                note(obj);
            }
            Instr::Append { obj, val } => {
                note(obj);
                note(val);
            }
            Instr::GetGlobal { dst, .. } => note(dst),
            Instr::SetGlobal { src, .. } => note(src),
            Instr::GetUpval { dst, .. } => note(dst),
            Instr::SetUpval { src, .. } => note(src),
            Instr::Closure { dst, .. } => note(dst),
            Instr::Jump { .. } | Instr::Nop | Instr::Halt => {}
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(src: &str) -> Module {
        let program = crate::parser::parse(crate::lexer::tokenize(src).unwrap()).unwrap();
        crate::compiler::compile(&program, "t").unwrap()
    }

    #[test]
    fn stats_count_instructions() {
        let m = compile("let x = 1 + 2; return x;");
        let s = module_stats(&m);
        assert_eq!(s.proto_count, 1);
        assert!(s.total_instructions >= 3);
        assert!(s.opcode_histogram.contains_key("add"));
    }

    #[test]
    fn closures_are_reachable_and_counted() {
        let m = compile("func f() { return func() { return 1; }; } return f();");
        let reachable = reachable_protos(&m);
        assert_eq!(reachable.len(), m.protos.len());
        assert!(unreachable_protos(&m).is_empty());
    }

    #[test]
    fn complexity_tracks_branches() {
        let m = compile("let i = 0; while (i < 3) { i = i + 1; } return i;");
        let c = &module_complexity(&m)[0];
        assert!(c.branches >= 1);
        assert_eq!(c.cyclomatic, c.branches + 1);
    }

    #[test]
    fn max_register_used_within_declared_count() {
        let m = compile("let a = 1; let b = 2; let c = a + b; return c;");
        for proto in &m.protos {
            assert!(max_register_used(proto) <= proto.reg_count as u32);
        }
    }
}
