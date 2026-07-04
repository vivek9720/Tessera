//! A recursive-descent parser with precedence climbing for expressions.
//!
//! The parser consumes the token vector produced by [`crate::lexer`] and builds
//! the [`crate::ast`]. Expression precedence is handled by a binding-power table
//! ([`infix_binding_power`]); statements are parsed by straightforward recursive
//! descent. A depth counter guards against deeply nested input driving the
//! parser into a native stack overflow — nesting past the limit is a parse
//! error, not a crash.

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, LogicOp, Program, Stmt, StmtKind, UnOp};
use crate::error::{Error, Result, Span};
use crate::token::{TokKind, Token};

/// Maximum expression/statement nesting depth.
const MAX_DEPTH: u32 = 200;

/// Parse a full program from source tokens.
pub fn parse(tokens: Vec<Token>) -> Result<Program> {
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    p.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: u32,
}

impl Parser {
    // ---- Token cursor ------------------------------------------------------

    fn peek(&self) -> &Token {
        // The lexer guarantees a trailing Eof, so this never indexes past the
        // end for a well-formed token vector; clamp defensively regardless.
        let idx = self.pos.min(self.tokens.len().saturating_sub(1));
        &self.tokens[idx]
    }

    fn peek_kind(&self) -> &TokKind {
        &self.peek().kind
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn check(&self, kind: &TokKind) -> bool {
        self.peek_kind() == kind
    }

    fn eat(&mut self, kind: &TokKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokKind) -> Result<Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            Err(Error::parse(
                self.span(),
                format!(
                    "expected {}, found {}",
                    kind.describe(),
                    self.peek_kind().describe()
                ),
            ))
        }
    }

    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::parse(self.span(), "expression nested too deeply"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ---- Program and statements -------------------------------------------

    fn parse_program(&mut self) -> Result<Program> {
        let mut body = Vec::new();
        while !self.check(&TokKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        Ok(Program { body })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.enter()?;
        self.expect(TokKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.check(&TokKind::RBrace) && !self.check(&TokKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(TokKind::RBrace)?;
        self.leave();
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        self.enter()?;
        let start = self.span();
        let stmt = match self.peek_kind() {
            TokKind::Let => self.parse_let(start)?,
            TokKind::Func => self.parse_func_decl(start)?,
            TokKind::Return => self.parse_return(start)?,
            TokKind::If => self.parse_if(start)?,
            TokKind::While => self.parse_while(start)?,
            TokKind::Break => {
                self.advance();
                self.expect(TokKind::Semicolon)?;
                Stmt::new(StmtKind::Break, start)
            }
            TokKind::Continue => {
                self.advance();
                self.expect(TokKind::Semicolon)?;
                Stmt::new(StmtKind::Continue, start)
            }
            _ => self.parse_expr_or_assign(start)?,
        };
        self.leave();
        Ok(stmt)
    }

    fn parse_let(&mut self, start: Span) -> Result<Stmt> {
        self.expect(TokKind::Let)?;
        let name = self.parse_ident_name()?;
        self.expect(TokKind::Assign)?;
        let value = self.parse_expr(0)?;
        self.expect(TokKind::Semicolon)?;
        Ok(Stmt::new(StmtKind::Let(name, value), start))
    }

    fn parse_func_decl(&mut self, start: Span) -> Result<Stmt> {
        self.expect(TokKind::Func)?;
        let name = self.parse_ident_name()?;
        let def = self.parse_func_rest(Some(name.clone()), start)?;
        Ok(Stmt::new(StmtKind::Func(name, Box::new(def)), start))
    }

    fn parse_return(&mut self, start: Span) -> Result<Stmt> {
        self.expect(TokKind::Return)?;
        if self.eat(&TokKind::Semicolon) {
            return Ok(Stmt::new(StmtKind::Return(None), start));
        }
        let value = self.parse_expr(0)?;
        self.expect(TokKind::Semicolon)?;
        Ok(Stmt::new(StmtKind::Return(Some(value)), start))
    }

    fn parse_if(&mut self, start: Span) -> Result<Stmt> {
        self.expect(TokKind::If)?;
        self.expect(TokKind::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(TokKind::RParen)?;
        let then_block = self.parse_block()?;
        let else_block = if self.eat(&TokKind::Else) {
            if self.check(&TokKind::If) {
                // `else if` chains as a single nested statement.
                let nested_start = self.span();
                vec![self.parse_if(nested_start)?]
            } else {
                self.parse_block()?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt::new(StmtKind::If(cond, then_block, else_block), start))
    }

    fn parse_while(&mut self, start: Span) -> Result<Stmt> {
        self.expect(TokKind::While)?;
        self.expect(TokKind::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(TokKind::RParen)?;
        let body = self.parse_block()?;
        Ok(Stmt::new(StmtKind::While(cond, body), start))
    }

    fn parse_expr_or_assign(&mut self, start: Span) -> Result<Stmt> {
        let lhs = self.parse_expr(0)?;
        if self.eat(&TokKind::Assign) {
            let rhs = self.parse_expr(0)?;
            self.expect(TokKind::Semicolon)?;
            let stmt = match lhs.kind {
                ExprKind::Ident(name) => StmtKind::Assign(name, rhs),
                ExprKind::Index(obj, key) => StmtKind::AssignIndex(*obj, *key, rhs),
                _ => {
                    return Err(Error::parse(
                        lhs.span,
                        "left-hand side of assignment is not assignable",
                    ))
                }
            };
            Ok(Stmt::new(stmt, start))
        } else {
            self.expect(TokKind::Semicolon)?;
            Ok(Stmt::new(StmtKind::ExprStmt(lhs), start))
        }
    }

    // ---- Expressions -------------------------------------------------------

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr> {
        self.enter()?;
        let mut lhs = self.parse_unary()?;

        loop {
            let op = match infix_op(self.peek_kind()) {
                Some(op) => op,
                None => break,
            };
            let (lbp, rbp) = infix_binding_power(op);
            if lbp < min_bp {
                break;
            }
            let op_span = self.span();
            self.advance();
            let rhs = self.parse_expr(rbp)?;
            let span = lhs.span.merge(rhs.span).merge(op_span);
            lhs = match op {
                InfixOp::Bin(b) => {
                    Expr::new(ExprKind::Binary(b, Box::new(lhs), Box::new(rhs)), span)
                }
                InfixOp::Logic(l) => {
                    Expr::new(ExprKind::Logical(l, Box::new(lhs), Box::new(rhs)), span)
                }
            };
        }

        self.leave();
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let start = self.span();
        let op = match self.peek_kind() {
            TokKind::Minus => Some(UnOp::Neg),
            TokKind::Bang | TokKind::Not => Some(UnOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let operand = self.parse_unary()?;
            let span = start.merge(operand.span);
            return Ok(Expr::new(ExprKind::Unary(op, Box::new(operand)), span));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokKind::LParen => {
                    let args = self.parse_call_args()?;
                    let span = expr.span.merge(self.prev_span());
                    expr = Expr::new(ExprKind::Call(Box::new(expr), args), span);
                }
                TokKind::LBracket => {
                    self.advance();
                    let key = self.parse_expr(0)?;
                    self.expect(TokKind::RBracket)?;
                    let span = expr.span.merge(key.span);
                    expr = Expr::new(ExprKind::Index(Box::new(expr), Box::new(key)), span);
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>> {
        self.expect(TokKind::LParen)?;
        let mut args = Vec::new();
        if !self.check(&TokKind::RParen) {
            loop {
                args.push(self.parse_expr(0)?);
                if !self.eat(&TokKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokKind::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let span = self.span();
        let kind = match self.peek_kind().clone() {
            TokKind::Int(i) => {
                self.advance();
                ExprKind::Int(i)
            }
            TokKind::Float(f) => {
                self.advance();
                ExprKind::Float(f)
            }
            TokKind::Str(s) => {
                self.advance();
                ExprKind::Str(s)
            }
            TokKind::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            TokKind::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            TokKind::Nil => {
                self.advance();
                ExprKind::Nil
            }
            TokKind::Ident(name) => {
                self.advance();
                ExprKind::Ident(name)
            }
            TokKind::LParen => {
                self.advance();
                let inner = self.parse_expr(0)?;
                self.expect(TokKind::RParen)?;
                return Ok(inner);
            }
            TokKind::LBracket => return self.parse_list(span),
            TokKind::LBrace => return self.parse_map(span),
            TokKind::Func => {
                self.advance();
                let def = self.parse_func_rest(None, span)?;
                ExprKind::Func(Box::new(def))
            }
            other => {
                return Err(Error::parse(
                    span,
                    format!("expected an expression, found {}", other.describe()),
                ))
            }
        };
        Ok(Expr::new(kind, span))
    }

    fn parse_list(&mut self, start: Span) -> Result<Expr> {
        self.expect(TokKind::LBracket)?;
        let mut items = Vec::new();
        if !self.check(&TokKind::RBracket) {
            loop {
                items.push(self.parse_expr(0)?);
                if !self.eat(&TokKind::Comma) {
                    break;
                }
                if self.check(&TokKind::RBracket) {
                    break; // allow a trailing comma
                }
            }
        }
        self.expect(TokKind::RBracket)?;
        Ok(Expr::new(ExprKind::List(items), start))
    }

    fn parse_map(&mut self, start: Span) -> Result<Expr> {
        self.expect(TokKind::LBrace)?;
        let mut pairs = Vec::new();
        if !self.check(&TokKind::RBrace) {
            loop {
                let key = self.parse_expr(0)?;
                self.expect(TokKind::Colon)?;
                let value = self.parse_expr(0)?;
                pairs.push((key, value));
                if !self.eat(&TokKind::Comma) {
                    break;
                }
                if self.check(&TokKind::RBrace) {
                    break;
                }
            }
        }
        self.expect(TokKind::RBrace)?;
        Ok(Expr::new(ExprKind::Map(pairs), start))
    }

    fn parse_func_rest(&mut self, name: Option<String>, start: Span) -> Result<FuncDef> {
        self.expect(TokKind::LParen)?;
        let mut params = Vec::new();
        if !self.check(&TokKind::RParen) {
            loop {
                params.push(self.parse_ident_name()?);
                if !self.eat(&TokKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokKind::RParen)?;
        let body = self.parse_block()?;
        Ok(FuncDef {
            name,
            params,
            body,
            span: start,
        })
    }

    fn parse_ident_name(&mut self) -> Result<String> {
        match self.peek_kind().clone() {
            TokKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(Error::parse(
                self.span(),
                format!("expected an identifier, found {}", other.describe()),
            )),
        }
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            self.span()
        } else {
            self.tokens[self.pos - 1].span
        }
    }
}

/// An infix operator resolved from a token.
#[derive(Clone, Copy)]
enum InfixOp {
    Bin(BinOp),
    Logic(LogicOp),
}

fn infix_op(kind: &TokKind) -> Option<InfixOp> {
    let op = match kind {
        TokKind::Or => InfixOp::Logic(LogicOp::Or),
        TokKind::And => InfixOp::Logic(LogicOp::And),
        TokKind::EqEq => InfixOp::Bin(BinOp::Eq),
        TokKind::BangEq => InfixOp::Bin(BinOp::Ne),
        TokKind::Lt => InfixOp::Bin(BinOp::Lt),
        TokKind::Le => InfixOp::Bin(BinOp::Le),
        TokKind::Gt => InfixOp::Bin(BinOp::Gt),
        TokKind::Ge => InfixOp::Bin(BinOp::Ge),
        TokKind::PlusPlus => InfixOp::Bin(BinOp::Concat),
        TokKind::Plus => InfixOp::Bin(BinOp::Add),
        TokKind::Minus => InfixOp::Bin(BinOp::Sub),
        TokKind::Star => InfixOp::Bin(BinOp::Mul),
        TokKind::Slash => InfixOp::Bin(BinOp::Div),
        TokKind::Percent => InfixOp::Bin(BinOp::Mod),
        TokKind::StarStar => InfixOp::Bin(BinOp::Pow),
        _ => return None,
    };
    Some(op)
}

/// Left/right binding powers. A right power lower than the left makes the
/// operator right-associative (used by `**`).
fn infix_binding_power(op: InfixOp) -> (u8, u8) {
    match op {
        InfixOp::Logic(LogicOp::Or) => (1, 2),
        InfixOp::Logic(LogicOp::And) => (3, 4),
        InfixOp::Bin(BinOp::Eq) | InfixOp::Bin(BinOp::Ne) => (5, 6),
        InfixOp::Bin(BinOp::Lt)
        | InfixOp::Bin(BinOp::Le)
        | InfixOp::Bin(BinOp::Gt)
        | InfixOp::Bin(BinOp::Ge) => (7, 8),
        InfixOp::Bin(BinOp::Concat) => (9, 10),
        InfixOp::Bin(BinOp::Add) | InfixOp::Bin(BinOp::Sub) => (11, 12),
        InfixOp::Bin(BinOp::Mul) | InfixOp::Bin(BinOp::Div) | InfixOp::Bin(BinOp::Mod) => (13, 14),
        InfixOp::Bin(BinOp::Pow) => (18, 17),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse_src(src: &str) -> Result<Program> {
        parse(tokenize(src).unwrap())
    }

    #[test]
    fn parses_let_and_return() {
        let p = parse_src("let x = 1 + 2; return x;").unwrap();
        assert_eq!(p.body.len(), 2);
    }

    #[test]
    fn precedence_is_respected() {
        // 1 + 2 * 3 should parse as 1 + (2 * 3)
        let p = parse_src("return 1 + 2 * 3;").unwrap();
        match &p.body[0].kind {
            StmtKind::Return(Some(e)) => match &e.kind {
                ExprKind::Binary(BinOp::Add, _, rhs) => {
                    assert!(matches!(rhs.kind, ExprKind::Binary(BinOp::Mul, _, _)));
                }
                other => panic!("unexpected: {other:?}"),
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_functions_and_closures() {
        let src = "func adder(x) { return func(y) { return x + y; }; }";
        assert!(parse_src(src).is_ok());
    }

    #[test]
    fn rejects_deep_nesting() {
        let src = format!("return {};", "(".repeat(500));
        assert!(parse_src(&src).is_err());
    }

    #[test]
    fn rejects_bad_assignment_target() {
        assert!(parse_src("1 + 2 = 3;").is_err());
    }
}
