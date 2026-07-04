//! Token definitions for the Tessera lexer.

use crate::error::Span;

/// A lexical token kind. Literals carry their decoded payload so the parser does
/// not re-scan the source text.
#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    // Literals
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),

    // Keywords
    Let,
    Func,
    Return,
    If,
    Else,
    While,
    For,
    In,
    True,
    False,
    Nil,
    And,
    Or,
    Not,
    Break,
    Continue,

    // Punctuation and operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    Dot,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    Bang,
    PlusPlus,

    /// End of input.
    Eof,
}

impl TokKind {
    /// A human-readable description used in parser error messages.
    pub fn describe(&self) -> String {
        match self {
            TokKind::Int(_) => "integer literal".to_string(),
            TokKind::Float(_) => "float literal".to_string(),
            TokKind::Str(_) => "string literal".to_string(),
            TokKind::Ident(name) => format!("identifier `{name}`"),
            TokKind::Eof => "end of input".to_string(),
            other => format!("`{}`", other.symbol()),
        }
    }

    /// The canonical spelling for keyword and punctuation tokens.
    pub fn symbol(&self) -> &'static str {
        match self {
            TokKind::Let => "let",
            TokKind::Func => "func",
            TokKind::Return => "return",
            TokKind::If => "if",
            TokKind::Else => "else",
            TokKind::While => "while",
            TokKind::For => "for",
            TokKind::In => "in",
            TokKind::True => "true",
            TokKind::False => "false",
            TokKind::Nil => "nil",
            TokKind::And => "and",
            TokKind::Or => "or",
            TokKind::Not => "not",
            TokKind::Break => "break",
            TokKind::Continue => "continue",
            TokKind::LParen => "(",
            TokKind::RParen => ")",
            TokKind::LBrace => "{",
            TokKind::RBrace => "}",
            TokKind::LBracket => "[",
            TokKind::RBracket => "]",
            TokKind::Comma => ",",
            TokKind::Semicolon => ";",
            TokKind::Colon => ":",
            TokKind::Dot => ".",
            TokKind::Assign => "=",
            TokKind::Plus => "+",
            TokKind::Minus => "-",
            TokKind::Star => "*",
            TokKind::Slash => "/",
            TokKind::Percent => "%",
            TokKind::StarStar => "**",
            TokKind::EqEq => "==",
            TokKind::BangEq => "!=",
            TokKind::Lt => "<",
            TokKind::Le => "<=",
            TokKind::Gt => ">",
            TokKind::Ge => ">=",
            TokKind::Bang => "!",
            TokKind::PlusPlus => "++",
            _ => "?",
        }
    }
}

/// Map an identifier to its keyword kind, or `None` if it is a plain name.
pub fn keyword(ident: &str) -> Option<TokKind> {
    let kind = match ident {
        "let" => TokKind::Let,
        "func" => TokKind::Func,
        "return" => TokKind::Return,
        "if" => TokKind::If,
        "else" => TokKind::Else,
        "while" => TokKind::While,
        "for" => TokKind::For,
        "in" => TokKind::In,
        "true" => TokKind::True,
        "false" => TokKind::False,
        "nil" => TokKind::Nil,
        "and" => TokKind::And,
        "or" => TokKind::Or,
        "not" => TokKind::Not,
        "break" => TokKind::Break,
        "continue" => TokKind::Continue,
        _ => return None,
    };
    Some(kind)
}

/// A token with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokKind, span: Span) -> Token {
        Token { kind, span }
    }
}
