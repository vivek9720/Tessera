//! The Tessera lexer: source text to a token stream.
//!
//! The lexer is a straightforward hand-written scanner. It tracks line and
//! column positions so tokens carry accurate [`Span`]s, decodes numeric and
//! string literals eagerly, and reports the first malformed construct it sees as
//! a [`ErrorKind::Lex`](crate::error::ErrorKind::Lex) error.

use crate::error::{Error, Result, Span};
use crate::token::{keyword, TokKind, Token};

/// Tokenize `src` into a vector ending with a single [`TokKind::Eof`].
pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    Lexer::new(src).run()
}

struct Lexer<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Lexer<'a> {
        Lexer {
            src: src.as_bytes(),
            text: src,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn run(mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.at_end() {
                tokens.push(Token::new(TokKind::Eof, self.here(0)));
                break;
            }
            let tok = self.next_token()?;
            tokens.push(tok);
            // A generous cap so a pathological input cannot produce an unbounded
            // token vector; real programs never approach it.
            if tokens.len() > 4_000_000 {
                return Err(Error::lex(self.here(0), "token limit exceeded"));
            }
        }
        Ok(tokens)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> u8 {
        if self.pos < self.src.len() {
            self.src[self.pos]
        } else {
            0
        }
    }

    fn peek2(&self) -> u8 {
        if self.pos + 1 < self.src.len() {
            self.src[self.pos + 1]
        } else {
            0
        }
    }

    fn bump(&mut self) -> u8 {
        let c = self.peek();
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn here(&self, len: u32) -> Span {
        Span::new(self.line, self.col, self.pos as u32, len)
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            let c = self.peek();
            match c {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.bump();
                }
                b'#' => {
                    // Line comment.
                    while !self.at_end() && self.peek() != b'\n' {
                        self.bump();
                    }
                }
                b'/' if self.peek2() == b'/' => {
                    while !self.at_end() && self.peek() != b'\n' {
                        self.bump();
                    }
                }
                b'/' if self.peek2() == b'*' => {
                    self.bump();
                    self.bump();
                    let start = self.here(0);
                    loop {
                        if self.at_end() {
                            return Err(Error::lex(start, "unterminated block comment"));
                        }
                        if self.peek() == b'*' && self.peek2() == b'/' {
                            self.bump();
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn next_token(&mut self) -> Result<Token> {
        let start_line = self.line;
        let start_col = self.col;
        let start_pos = self.pos;
        let c = self.peek();

        let kind = if c == b'"' {
            self.lex_string()?
        } else if c.is_ascii_digit() {
            self.lex_number()?
        } else if is_ident_start(c) {
            self.lex_ident_or_keyword()
        } else {
            self.lex_symbol()?
        };

        let span = Span::new(
            start_line,
            start_col,
            start_pos as u32,
            (self.pos - start_pos) as u32,
        );
        Ok(Token::new(kind, span))
    }

    fn lex_string(&mut self) -> Result<TokKind> {
        let open = self.here(0);
        self.bump(); // consume opening quote
        let mut out = String::new();
        loop {
            if self.at_end() {
                return Err(Error::lex(open, "unterminated string literal"));
            }
            let c = self.bump();
            match c {
                b'"' => break,
                b'\\' => {
                    if self.at_end() {
                        return Err(Error::lex(open, "unterminated escape sequence"));
                    }
                    let esc = self.bump();
                    match esc {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'0' => out.push('\0'),
                        b'\\' => out.push('\\'),
                        b'"' => out.push('"'),
                        b'u' => self.lex_unicode_escape(&mut out, open)?,
                        other => {
                            return Err(Error::lex(
                                open,
                                format!("unknown escape `\\{}`", other as char),
                            ))
                        }
                    }
                }
                _ => {
                    // Re-decode this byte plus any UTF-8 continuation bytes.
                    self.push_utf8(c, &mut out)?;
                }
            }
        }
        Ok(TokKind::Str(out))
    }

    fn lex_unicode_escape(&mut self, out: &mut String, open: Span) -> Result<()> {
        if self.bump() != b'{' {
            return Err(Error::lex(open, "expected `{` after `\\u`"));
        }
        let mut code: u32 = 0;
        let mut digits = 0;
        while self.peek() != b'}' {
            if self.at_end() || digits >= 6 {
                return Err(Error::lex(open, "malformed unicode escape"));
            }
            let d = self.bump();
            let v = (d as char)
                .to_digit(16)
                .ok_or_else(|| Error::lex(open, "invalid hex digit in unicode escape"))?;
            code = code * 16 + v;
            digits += 1;
        }
        self.bump(); // consume `}`
        let ch = char::from_u32(code)
            .ok_or_else(|| Error::lex(open, "unicode escape is not a valid code point"))?;
        out.push(ch);
        Ok(())
    }

    fn push_utf8(&mut self, first: u8, out: &mut String) -> Result<()> {
        // Determine how many continuation bytes follow the lead byte.
        let extra = if first < 0x80 {
            0
        } else if first >> 5 == 0b110 {
            1
        } else if first >> 4 == 0b1110 {
            2
        } else if first >> 3 == 0b11110 {
            3
        } else {
            return Err(Error::lex(self.here(0), "invalid UTF-8 lead byte in string"));
        };
        let begin = self.pos - 1;
        for _ in 0..extra {
            if self.at_end() {
                return Err(Error::lex(self.here(0), "truncated UTF-8 sequence"));
            }
            self.bump();
        }
        let slice = &self.text[begin..self.pos];
        match slice.chars().next() {
            Some(ch) => {
                out.push(ch);
                Ok(())
            }
            None => Err(Error::lex(self.here(0), "invalid UTF-8 sequence in string")),
        }
    }

    fn lex_number(&mut self) -> Result<TokKind> {
        let start = self.pos;
        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.bump();
        }
        let mut is_float = false;
        if self.peek() == b'.' && self.peek2().is_ascii_digit() {
            is_float = true;
            self.bump();
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.bump();
            }
        }
        if self.peek() == b'e' || self.peek() == b'E' {
            is_float = true;
            self.bump();
            if self.peek() == b'+' || self.peek() == b'-' {
                self.bump();
            }
            if !self.peek().is_ascii_digit() {
                return Err(Error::lex(self.here(0), "malformed exponent"));
            }
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }
        let raw: String = self.text[start..self.pos].chars().filter(|c| *c != '_').collect();
        if is_float {
            raw.parse::<f64>()
                .map(TokKind::Float)
                .map_err(|_| Error::lex(self.here(0), "invalid float literal"))
        } else {
            match raw.parse::<i64>() {
                Ok(i) => Ok(TokKind::Int(i)),
                // Fall back to float for out-of-range integers.
                Err(_) => raw
                    .parse::<f64>()
                    .map(TokKind::Float)
                    .map_err(|_| Error::lex(self.here(0), "integer literal out of range")),
            }
        }
    }

    fn lex_ident_or_keyword(&mut self) -> TokKind {
        let start = self.pos;
        while is_ident_continue(self.peek()) {
            self.bump();
        }
        let name = &self.text[start..self.pos];
        keyword(name).unwrap_or_else(|| TokKind::Ident(name.to_string()))
    }

    fn lex_symbol(&mut self) -> Result<TokKind> {
        let span = self.here(0);
        let c = self.bump();
        let kind = match c {
            b'(' => TokKind::LParen,
            b')' => TokKind::RParen,
            b'{' => TokKind::LBrace,
            b'}' => TokKind::RBrace,
            b'[' => TokKind::LBracket,
            b']' => TokKind::RBracket,
            b',' => TokKind::Comma,
            b';' => TokKind::Semicolon,
            b':' => TokKind::Colon,
            b'.' => TokKind::Dot,
            b'+' => {
                if self.peek() == b'+' {
                    self.bump();
                    TokKind::PlusPlus
                } else {
                    TokKind::Plus
                }
            }
            b'-' => TokKind::Minus,
            b'*' => {
                if self.peek() == b'*' {
                    self.bump();
                    TokKind::StarStar
                } else {
                    TokKind::Star
                }
            }
            b'/' => TokKind::Slash,
            b'%' => TokKind::Percent,
            b'=' => {
                if self.peek() == b'=' {
                    self.bump();
                    TokKind::EqEq
                } else {
                    TokKind::Assign
                }
            }
            b'!' => {
                if self.peek() == b'=' {
                    self.bump();
                    TokKind::BangEq
                } else {
                    TokKind::Bang
                }
            }
            b'<' => {
                if self.peek() == b'=' {
                    self.bump();
                    TokKind::Le
                } else {
                    TokKind::Lt
                }
            }
            b'>' => {
                if self.peek() == b'=' {
                    self.bump();
                    TokKind::Ge
                } else {
                    TokKind::Gt
                }
            }
            other => {
                return Err(Error::lex(
                    span,
                    format!("unexpected character `{}`", other as char),
                ))
            }
        };
        Ok(kind)
    }
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_arithmetic() {
        let k = kinds("1 + 2 * 3");
        assert_eq!(
            k,
            vec![
                TokKind::Int(1),
                TokKind::Plus,
                TokKind::Int(2),
                TokKind::Star,
                TokKind::Int(3),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_keywords_and_idents() {
        let k = kinds("let x = func");
        assert_eq!(k[0], TokKind::Let);
        assert_eq!(k[1], TokKind::Ident("x".to_string()));
        assert_eq!(k[2], TokKind::Assign);
        assert_eq!(k[3], TokKind::Func);
    }

    #[test]
    fn lexes_strings_with_escapes() {
        let k = kinds(r#""a\nb\u{41}""#);
        assert_eq!(k[0], TokKind::Str("a\nbA".to_string()));
    }

    #[test]
    fn floats_and_underscores() {
        assert_eq!(kinds("1_000")[0], TokKind::Int(1000));
        assert_eq!(kinds("3.5e2")[0], TokKind::Float(350.0));
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(tokenize("\"oops").is_err());
    }
}
