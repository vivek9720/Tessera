//! Formatting utilities: pretty-print an AST back to source, and summarize a
//! compiled module as JSON.
//!
//! The pretty-printer is used by the `tessera fmt` subcommand and by tests to
//! confirm the parser produced the expected structure. The JSON summary offers
//! a machine-readable view of a module's shape for external tooling. Neither
//! touches the runtime heap; they work purely on the AST and [`Module`].

use std::fmt::Write as _;

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, LogicOp, Program, Stmt, StmtKind, UnOp};
use crate::module::{Const, Module, UpvalDesc};

/// Pretty-print a parsed program to canonical Tessera source.
pub fn pretty_program(program: &Program) -> String {
    let mut p = Printer { out: String::new(), indent: 0 };
    for stmt in &program.body {
        p.stmt(stmt);
    }
    p.out
}

struct Printer {
    out: String,
    indent: usize,
}

impl Printer {
    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }

    fn line(&mut self, s: &str) {
        self.pad();
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let(name, expr) => {
                let e = self.expr(expr);
                self.line(&format!("let {name} = {e};"));
            }
            StmtKind::Assign(name, expr) => {
                let e = self.expr(expr);
                self.line(&format!("{name} = {e};"));
            }
            StmtKind::AssignIndex(obj, key, val) => {
                let o = self.expr(obj);
                let k = self.expr(key);
                let v = self.expr(val);
                self.line(&format!("{o}[{k}] = {v};"));
            }
            StmtKind::ExprStmt(expr) => {
                let e = self.expr(expr);
                self.line(&format!("{e};"));
            }
            StmtKind::Return(opt) => match opt {
                Some(expr) => {
                    let e = self.expr(expr);
                    self.line(&format!("return {e};"));
                }
                None => self.line("return;"),
            },
            StmtKind::If(cond, then_block, else_block) => {
                let c = self.expr(cond);
                self.line(&format!("if ({c}) {{"));
                self.indent += 1;
                for s in then_block {
                    self.stmt(s);
                }
                self.indent -= 1;
                if else_block.is_empty() {
                    self.line("}");
                } else {
                    self.line("} else {");
                    self.indent += 1;
                    for s in else_block {
                        self.stmt(s);
                    }
                    self.indent -= 1;
                    self.line("}");
                }
            }
            StmtKind::While(cond, body) => {
                let c = self.expr(cond);
                self.line(&format!("while ({c}) {{"));
                self.indent += 1;
                for s in body {
                    self.stmt(s);
                }
                self.indent -= 1;
                self.line("}");
            }
            StmtKind::Func(name, def) => {
                let header = format!("func {name}({}) {{", def.params.join(", "));
                self.line(&header);
                self.indent += 1;
                for s in &def.body {
                    self.stmt(s);
                }
                self.indent -= 1;
                self.line("}");
            }
            StmtKind::Break => self.line("break;"),
            StmtKind::Continue => self.line("continue;"),
        }
    }

    fn expr(&self, expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Nil => "nil".to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Int(i) => i.to_string(),
            ExprKind::Float(f) => format!("{f}"),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Ident(name) => name.clone(),
            ExprKind::Unary(op, e) => {
                let inner = self.expr(e);
                match op {
                    UnOp::Neg => format!("-{inner}"),
                    UnOp::Not => format!("not {inner}"),
                }
            }
            ExprKind::Binary(op, a, b) => {
                format!("({} {} {})", self.expr(a), bin_symbol(*op), self.expr(b))
            }
            ExprKind::Logical(op, a, b) => {
                let sym = match op {
                    LogicOp::And => "and",
                    LogicOp::Or => "or",
                };
                format!("({} {sym} {})", self.expr(a), self.expr(b))
            }
            ExprKind::Call(callee, args) => {
                let a: Vec<String> = args.iter().map(|e| self.expr(e)).collect();
                format!("{}({})", self.expr(callee), a.join(", "))
            }
            ExprKind::Index(obj, key) => format!("{}[{}]", self.expr(obj), self.expr(key)),
            ExprKind::List(items) => {
                let a: Vec<String> = items.iter().map(|e| self.expr(e)).collect();
                format!("[{}]", a.join(", "))
            }
            ExprKind::Map(pairs) => {
                let a: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.expr(k), self.expr(v)))
                    .collect();
                format!("{{{}}}", a.join(", "))
            }
            ExprKind::Func(def) => self.func_expr(def),
        }
    }

    fn func_expr(&self, def: &FuncDef) -> String {
        // Anonymous functions print their signature and a body placeholder so the
        // one-line expression formatter stays single-line; full bodies are shown
        // by the statement printer for named functions.
        let params = def.params.join(", ");
        format!("func({params}) {{ ... }}")
    }
}

fn bin_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Concat => "++",
    }
}

/// Serialize a module's structure as a JSON object (values are escaped).
pub fn module_to_json(module: &Module) -> String {
    let mut out = String::new();
    out.push('{');
    let _ = write!(out, "\"name\":{},", json_str(&module.name));
    let _ = write!(out, "\"version\":{},", module.version);
    let _ = write!(out, "\"entry\":{},", module.entry);

    out.push_str("\"consts\":[");
    for (i, c) in module.consts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&const_to_json(c));
    }
    out.push_str("],");

    out.push_str("\"protos\":[");
    for (i, proto) in module.protos.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        let _ = write!(out, "\"name\":{},", json_str(&proto.name));
        let _ = write!(out, "\"arity\":{},", proto.arity);
        let _ = write!(out, "\"regs\":{},", proto.reg_count);
        let _ = write!(out, "\"instructions\":{},", proto.code.len());
        out.push_str("\"upvalues\":[");
        for (j, uv) in proto.upvals.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            match uv {
                UpvalDesc::FromLocal(r) => {
                    let _ = write!(out, "{{\"local\":{r}}}");
                }
                UpvalDesc::FromUpval(idx) => {
                    let _ = write!(out, "{{\"parent\":{idx}}}");
                }
            }
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}

fn const_to_json(c: &Const) -> String {
    match c {
        Const::Int(v) => format!("{{\"int\":{v}}}"),
        Const::Float(v) => format!("{{\"float\":{v}}}"),
        Const::Str(s) => format!("{{\"str\":{}}}", json_str(s)),
        Const::Bytes(b) => format!("{{\"bytes\":{}}}", b.len()),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    #[test]
    fn pretty_prints_and_reparses() {
        let src = "let x = 1 + 2 * 3; if (x > 5) { return x; } else { return 0; }";
        let program = parse(tokenize(src).unwrap()).unwrap();
        let printed = pretty_program(&program);
        // The pretty-printed form must itself parse.
        assert!(parse(tokenize(&printed).unwrap()).is_ok());
        assert!(printed.contains("let x ="));
        assert!(printed.contains("if ("));
    }

    #[test]
    fn json_summary_is_well_formed() {
        let program = parse(tokenize("func f(x) { return x; } return f(1);").unwrap()).unwrap();
        let module = crate::compiler::compile(&program, "m").unwrap();
        let json = module_to_json(&module);
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"protos\""));
        assert!(json.contains("\"entry\""));
    }
}
