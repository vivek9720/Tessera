//! A conservative constant-folding pass over the AST.
//!
//! This runs before compilation and replaces expressions whose value is known
//! at compile time with the literal result — for example `2 + 3 * 4` becomes
//! `14`, and `"a" ++ "b"` becomes `"ab"`. The fold is deliberately narrow: it
//! only rewrites operations whose runtime semantics it can reproduce exactly
//! (wrapping integer add/sub/mul, integer comparisons, boolean negation, string
//! concatenation of literals). Anything that could diverge from the VM — integer
//! division and modulo (which can fault), floating point (rounding, NaN), and
//! power — is left untouched so that optimized and unoptimized programs always
//! agree.
//!
//! The pass is opt-in via [`crate::compile_source_optimized`]; the default
//! pipeline does not fold, keeping compilation output a direct mirror of the
//! source.

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, LogicOp, Program, Stmt, StmtKind, UnOp};

/// Fold constants throughout a program, returning a new program.
pub fn fold_program(program: &Program) -> Program {
    Program {
        body: program.body.iter().map(fold_stmt).collect(),
    }
}

fn fold_stmt(stmt: &Stmt) -> Stmt {
    let kind = match &stmt.kind {
        StmtKind::Let(name, expr) => StmtKind::Let(name.clone(), fold_expr(expr)),
        StmtKind::Assign(name, expr) => StmtKind::Assign(name.clone(), fold_expr(expr)),
        StmtKind::AssignIndex(obj, key, val) => {
            StmtKind::AssignIndex(fold_expr(obj), fold_expr(key), fold_expr(val))
        }
        StmtKind::ExprStmt(expr) => StmtKind::ExprStmt(fold_expr(expr)),
        StmtKind::Return(opt) => StmtKind::Return(opt.as_ref().map(fold_expr)),
        StmtKind::If(cond, then_block, else_block) => StmtKind::If(
            fold_expr(cond),
            then_block.iter().map(fold_stmt).collect(),
            else_block.iter().map(fold_stmt).collect(),
        ),
        StmtKind::While(cond, body) => {
            StmtKind::While(fold_expr(cond), body.iter().map(fold_stmt).collect())
        }
        StmtKind::Func(name, def) => StmtKind::Func(name.clone(), Box::new(fold_func(def))),
        StmtKind::Break => StmtKind::Break,
        StmtKind::Continue => StmtKind::Continue,
    };
    Stmt::new(kind, stmt.span)
}

fn fold_func(def: &FuncDef) -> FuncDef {
    FuncDef {
        name: def.name.clone(),
        params: def.params.clone(),
        body: def.body.iter().map(fold_stmt).collect(),
        span: def.span,
    }
}

fn fold_expr(expr: &Expr) -> Expr {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Unary(op, e) => {
            let inner = fold_expr(e);
            if let Some(k) = fold_unary(*op, &inner.kind) {
                return Expr::new(k, span);
            }
            Expr::new(ExprKind::Unary(*op, Box::new(inner)), span)
        }
        ExprKind::Binary(op, a, b) => {
            let fa = fold_expr(a);
            let fb = fold_expr(b);
            if let Some(k) = fold_binary(*op, &fa.kind, &fb.kind) {
                return Expr::new(k, span);
            }
            Expr::new(ExprKind::Binary(*op, Box::new(fa), Box::new(fb)), span)
        }
        ExprKind::Logical(op, a, b) => {
            let fa = fold_expr(a);
            let fb = fold_expr(b);
            // Fold only when the left operand is a constant that decides the
            // result, matching short-circuit semantics.
            match (op, &fa.kind) {
                (LogicOp::And, ExprKind::Bool(false)) => return Expr::new(ExprKind::Bool(false), span),
                (LogicOp::Or, ExprKind::Bool(true)) => return Expr::new(ExprKind::Bool(true), span),
                (LogicOp::And, ExprKind::Bool(true)) => return fb,
                (LogicOp::Or, ExprKind::Bool(false)) => return fb,
                _ => {}
            }
            Expr::new(ExprKind::Logical(*op, Box::new(fa), Box::new(fb)), span)
        }
        ExprKind::Call(callee, args) => Expr::new(
            ExprKind::Call(
                Box::new(fold_expr(callee)),
                args.iter().map(fold_expr).collect(),
            ),
            span,
        ),
        ExprKind::Index(obj, key) => Expr::new(
            ExprKind::Index(Box::new(fold_expr(obj)), Box::new(fold_expr(key))),
            span,
        ),
        ExprKind::List(items) => {
            Expr::new(ExprKind::List(items.iter().map(fold_expr).collect()), span)
        }
        ExprKind::Map(pairs) => Expr::new(
            ExprKind::Map(
                pairs
                    .iter()
                    .map(|(k, v)| (fold_expr(k), fold_expr(v)))
                    .collect(),
            ),
            span,
        ),
        ExprKind::Func(def) => Expr::new(ExprKind::Func(Box::new(fold_func(def))), span),
        // Literals and identifiers are already minimal.
        other => Expr::new(other.clone(), span),
    }
}

fn fold_unary(op: UnOp, operand: &ExprKind) -> Option<ExprKind> {
    match (op, operand) {
        (UnOp::Neg, ExprKind::Int(i)) => Some(ExprKind::Int(i.wrapping_neg())),
        (UnOp::Not, ExprKind::Bool(b)) => Some(ExprKind::Bool(!b)),
        (UnOp::Not, ExprKind::Nil) => Some(ExprKind::Bool(true)),
        _ => None,
    }
}

fn fold_binary(op: BinOp, a: &ExprKind, b: &ExprKind) -> Option<ExprKind> {
    match (a, b) {
        (ExprKind::Int(x), ExprKind::Int(y)) => fold_int(op, *x, *y),
        (ExprKind::Str(x), ExprKind::Str(y)) if op == BinOp::Concat => {
            Some(ExprKind::Str(format!("{x}{y}")))
        }
        _ => None,
    }
}

fn fold_int(op: BinOp, x: i64, y: i64) -> Option<ExprKind> {
    let result = match op {
        // Arithmetic that mirrors the VM's wrapping integer semantics.
        BinOp::Add => ExprKind::Int(x.wrapping_add(y)),
        BinOp::Sub => ExprKind::Int(x.wrapping_sub(y)),
        BinOp::Mul => ExprKind::Int(x.wrapping_mul(y)),
        // Comparisons produce booleans.
        BinOp::Eq => ExprKind::Bool(x == y),
        BinOp::Ne => ExprKind::Bool(x != y),
        BinOp::Lt => ExprKind::Bool(x < y),
        BinOp::Le => ExprKind::Bool(x <= y),
        BinOp::Gt => ExprKind::Bool(x > y),
        BinOp::Ge => ExprKind::Bool(x >= y),
        // Division, modulo, power, and concat over ints are intentionally not
        // folded (they can fault, overflow surprisingly, or change type).
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn fold_src(src: &str) -> Program {
        fold_program(&parse(tokenize(src).unwrap()).unwrap())
    }

    #[test]
    fn folds_integer_arithmetic() {
        let p = fold_src("return 2 + 3 * 4;");
        match &p.body[0].kind {
            StmtKind::Return(Some(e)) => assert!(matches!(e.kind, ExprKind::Int(14))),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn folds_string_concat() {
        let p = fold_src("return \"a\" ++ \"b\" ++ \"c\";");
        match &p.body[0].kind {
            StmtKind::Return(Some(e)) => {
                assert!(matches!(&e.kind, ExprKind::Str(s) if s == "abc"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn does_not_fold_division() {
        let p = fold_src("return 6 / 2;");
        match &p.body[0].kind {
            StmtKind::Return(Some(e)) => assert!(matches!(e.kind, ExprKind::Binary(BinOp::Div, _, _))),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn folds_comparisons() {
        let p = fold_src("return 3 < 5;");
        match &p.body[0].kind {
            StmtKind::Return(Some(e)) => assert!(matches!(e.kind, ExprKind::Bool(true))),
            other => panic!("unexpected {other:?}"),
        }
    }
}
