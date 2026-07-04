//! High-level inspection reports for compiled modules.
//!
//! The bytecode disassembler is intentionally close to the machine. This module
//! sits one level higher: it groups constants, call sites, closures, register
//! pressure, and validation hints into data structures that IDEs and command
//! line tools can consume without parsing text disassembly.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::{max_register_used, module_complexity, module_stats};
use crate::bytecode::Instr;
use crate::module::{Const, Module, Proto, UpvalDesc};

#[derive(Debug, Clone, Default)]
pub struct ModuleInspection {
    pub name: String,
    pub version: u16,
    pub entry: u32,
    pub constants: ConstantSummary,
    pub prototypes: Vec<PrototypeInspection>,
    pub call_graph: Vec<CallEdge>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConstantSummary {
    pub ints: usize,
    pub floats: usize,
    pub strings: usize,
    pub bytes: usize,
    pub total_string_bytes: usize,
    pub total_blob_bytes: usize,
    pub global_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PrototypeInspection {
    pub index: usize,
    pub name: String,
    pub arity: u16,
    pub variadic: bool,
    pub registers: u16,
    pub max_register_used: u32,
    pub instruction_count: usize,
    pub branch_count: usize,
    pub call_sites: usize,
    pub closure_count: usize,
    pub upvalues: Vec<UpvalueInspection>,
    pub opcode_counts: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpvalueInspection {
    pub index: usize,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    pub from: usize,
    pub to: Option<usize>,
    pub instruction: usize,
    pub kind: CallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    DirectClosure,
    DynamicValue,
    TailDynamic,
}

impl ModuleInspection {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "module {} v{} entry={}\n",
            if self.name.is_empty() { "<anonymous>" } else { &self.name },
            self.version,
            self.entry
        ));
        out.push_str(&format!(
            "constants: int={} float={} str={} bytes={} str_bytes={} blob_bytes={}\n",
            self.constants.ints,
            self.constants.floats,
            self.constants.strings,
            self.constants.bytes,
            self.constants.total_string_bytes,
            self.constants.total_blob_bytes
        ));
        if !self.constants.global_names.is_empty() {
            out.push_str("globals:");
            for name in &self.constants.global_names {
                out.push(' ');
                out.push_str(name);
            }
            out.push('\n');
        }
        for proto in &self.prototypes {
            out.push_str(&format!(
                "proto #{} {} arity={} regs={} used={} instr={} branches={} calls={} closures={}\n",
                proto.index,
                proto.name,
                proto.arity,
                proto.registers,
                proto.max_register_used,
                proto.instruction_count,
                proto.branch_count,
                proto.call_sites,
                proto.closure_count
            ));
            if !proto.upvalues.is_empty() {
                out.push_str("  upvalues:");
                for uv in &proto.upvalues {
                    out.push_str(&format!(" {}={}", uv.index, uv.source));
                }
                out.push('\n');
            }
        }
        if !self.call_graph.is_empty() {
            out.push_str("call graph:\n");
            for edge in &self.call_graph {
                let to = edge.to.map(|idx| idx.to_string()).unwrap_or_else(|| "dynamic".to_string());
                out.push_str(&format!(
                    "  #{} -> {} at {} ({:?})\n",
                    edge.from, to, edge.instruction, edge.kind
                ));
            }
        }
        for warning in &self.warnings {
            out.push_str("warning: ");
            out.push_str(warning);
            out.push('\n');
        }
        out
    }

    pub fn render_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        out.push_str(&format!(
            "\"name\":{},\"version\":{},\"entry\":{},",
            json(&self.name),
            self.version,
            self.entry
        ));
        out.push_str("\"constants\":{");
        out.push_str(&format!(
            "\"ints\":{},\"floats\":{},\"strings\":{},\"bytes\":{},\"string_bytes\":{},\"blob_bytes\":{}",
            self.constants.ints,
            self.constants.floats,
            self.constants.strings,
            self.constants.bytes,
            self.constants.total_string_bytes,
            self.constants.total_blob_bytes
        ));
        out.push_str("},\"prototypes\":[");
        for (idx, proto) in self.prototypes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"index\":{},\"name\":{},\"arity\":{},\"registers\":{},\"instructions\":{},\"branches\":{},\"calls\":{},\"closures\":{}}}",
                proto.index,
                json(&proto.name),
                proto.arity,
                proto.registers,
                proto.instruction_count,
                proto.branch_count,
                proto.call_sites,
                proto.closure_count
            ));
        }
        out.push_str("],\"warnings\":[");
        for (idx, warning) in self.warnings.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(&json(warning));
        }
        out.push_str("]}");
        out
    }
}

pub fn inspect_module(module: &Module) -> ModuleInspection {
    let stats = module_stats(module);
    let complexities = module_complexity(module);
    let mut inspection = ModuleInspection {
        name: module.name.clone(),
        version: module.version,
        entry: module.entry,
        constants: summarize_constants(module),
        prototypes: Vec::new(),
        call_graph: Vec::new(),
        warnings: Vec::new(),
    };

    for (idx, proto) in module.protos.iter().enumerate() {
        let complexity = &complexities[idx];
        inspection.prototypes.push(PrototypeInspection {
            index: idx,
            name: proto.name.clone(),
            arity: proto.arity,
            variadic: proto.is_variadic,
            registers: proto.reg_count,
            max_register_used: max_register_used(proto),
            instruction_count: proto.code.len(),
            branch_count: complexity.branches,
            call_sites: complexity.calls,
            closure_count: complexity.nested_closures,
            upvalues: inspect_upvalues(proto),
            opcode_counts: opcode_counts(proto),
        });
        inspection.call_graph.extend(call_edges(idx, proto));
    }

    if !stats.unreachable_protos.is_empty() {
        inspection
            .warnings
            .push(format!("unreachable prototypes: {:?}", stats.unreachable_protos));
    }
    for proto in &inspection.prototypes {
        if proto.max_register_used > proto.registers as u32 {
            inspection.warnings.push(format!(
                "prototype #{} touches register {} but declares {} registers",
                proto.index, proto.max_register_used, proto.registers
            ));
        }
        if proto.instruction_count == 0 {
            inspection
                .warnings
                .push(format!("prototype #{} has no instructions", proto.index));
        }
    }
    inspection
}

fn summarize_constants(module: &Module) -> ConstantSummary {
    let mut summary = ConstantSummary::default();
    let mut names = BTreeSet::new();
    for constant in &module.consts {
        match constant {
            Const::Int(_) => summary.ints += 1,
            Const::Float(_) => summary.floats += 1,
            Const::Str(value) => {
                summary.strings += 1;
                summary.total_string_bytes += value.len();
                if looks_like_global(value) {
                    names.insert(value.clone());
                }
            }
            Const::Bytes(value) => {
                summary.bytes += 1;
                summary.total_blob_bytes += value.len();
            }
        }
    }
    summary.global_names = names.into_iter().collect();
    summary
}

fn looks_like_global(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn inspect_upvalues(proto: &Proto) -> Vec<UpvalueInspection> {
    proto
        .upvals
        .iter()
        .enumerate()
        .map(|(idx, uv)| UpvalueInspection {
            index: idx,
            source: match uv {
                UpvalDesc::FromLocal(reg) => format!("local r{reg}"),
                UpvalDesc::FromUpval(parent) => format!("parent u{parent}"),
            },
        })
        .collect()
}

fn opcode_counts(proto: &Proto) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for instr in &proto.code {
        *counts.entry(instr.mnemonic()).or_insert(0) += 1;
    }
    counts
}

fn call_edges(from: usize, proto: &Proto) -> Vec<CallEdge> {
    let mut edges = Vec::new();
    for (pc, instr) in proto.code.iter().enumerate() {
        match *instr {
            Instr::Closure { proto, .. } => edges.push(CallEdge {
                from,
                to: Some(proto as usize),
                instruction: pc,
                kind: CallKind::DirectClosure,
            }),
            Instr::Call { .. } => edges.push(CallEdge {
                from,
                to: None,
                instruction: pc,
                kind: CallKind::DynamicValue,
            }),
            Instr::TailCall { .. } => edges.push(CallEdge {
                from,
                to: None,
                instruction: pc,
                kind: CallKind::TailDynamic,
            }),
            _ => {}
        }
    }
    edges
}

fn json(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn inspect_bytes(bytes: &[u8]) -> crate::Result<ModuleInspection> {
    let module = crate::load_module(bytes)?;
    Ok(inspect_module(&module))
}

pub fn inspect_source(source: &str, name: &str) -> crate::Result<ModuleInspection> {
    let module = crate::compile_source(source, name)?;
    Ok(inspect_module(&module))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_reports_basic_module_shape() {
        let module = crate::compile_source("func id(x) { return x; } return id(3);", "demo").unwrap();
        let report = inspect_module(&module);
        assert_eq!(report.name, "demo");
        assert!(!report.prototypes.is_empty());
        assert!(report.call_graph.iter().any(|edge| edge.kind == CallKind::DynamicValue));
    }

    #[test]
    fn json_report_is_object() {
        let report = inspect_source("return 1;", "one").unwrap();
        let json = report.render_json();
        assert!(json.starts_with('{'));
        assert!(json.contains("\"prototypes\""));
    }
}
