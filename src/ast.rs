//! The Tessera abstract syntax tree.
//!
//! The AST is intentionally small: a program is a list of [`Stmt`], and every
//! expression form the surface language supports is a variant of [`ExprKind`].
//! Nodes carry a [`Span`] so the compiler can attribute errors back to source.

use crate::error::Span;

/// Binary arithmetic, comparison, and concatenation operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
}

/// Short-circuiting logical operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
}

/// Prefix unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// An expression node.
#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span }
    }
}

/// The shape of an expression.
#[derive(Debug, Clone)]
pub enum ExprKind {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(LogicOp, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Func(Box<FuncDef>),
}

/// A function definition, shared by named declarations and anonymous closures.
#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// A statement node.
#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Stmt {
        Stmt { kind, span }
    }
}

/// The shape of a statement.
#[derive(Debug, Clone)]
pub enum StmtKind {
    /// `let name = expr;`
    Let(String, Expr),
    /// `name = expr;`
    Assign(String, Expr),
    /// `target[index] = value;`
    AssignIndex(Expr, Expr, Expr),
    /// A bare expression evaluated for its effects.
    ExprStmt(Expr),
    /// `return;` or `return expr;`
    Return(Option<Expr>),
    /// `if (cond) { then } else { otherwise }`
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    /// `while (cond) { body }`
    While(Expr, Vec<Stmt>),
    /// A named function declaration bound in the current scope.
    Func(String, Box<FuncDef>),
    /// `break;`
    Break,
    /// `continue;`
    Continue,
}

/// A whole program: the statements that become the body of `main`.
#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
}
