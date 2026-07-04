//! Error types shared across the Tessera front end, module loader, and virtual
//! machine.
//!
//! The crate uses a single [`Error`] enum with a coarse [`ErrorKind`]
//! classification. Every fallible operation returns [`Result`], and the runtime
//! is written so that *malformed but structurally valid* programs surface as
//! ordinary `Err` values rather than panics. That discipline keeps the failure
//! surface predictable for embedders that run untrusted bytecode.

use std::fmt;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Broad classification of an [`Error`].
///
/// The kind is deliberately coarse; the human-readable message carried by the
/// [`Error`] is where the detail lives. Embedders typically switch on the kind
/// to decide whether a failure is the caller's fault (a bad module) or the
/// script's fault (a runtime fault).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The source text could not be tokenized.
    Lex,
    /// The token stream did not form a valid program.
    Parse,
    /// The abstract syntax tree could not be lowered to bytecode.
    Compile,
    /// A serialized module was structurally invalid or truncated.
    Decode,
    /// A module decoded but failed a semantic verification pass.
    Verify,
    /// The virtual machine hit a fault while executing valid bytecode.
    Runtime,
    /// A value had the wrong type for the requested operation.
    Type,
    /// An index, arity, or capacity limit was exceeded.
    Bounds,
    /// The host refused an operation (unknown native, disabled feature).
    Host,
}

impl ErrorKind {
    /// A short, stable, lowercase tag suitable for logs.
    pub fn tag(self) -> &'static str {
        match self {
            ErrorKind::Lex => "lex",
            ErrorKind::Parse => "parse",
            ErrorKind::Compile => "compile",
            ErrorKind::Decode => "decode",
            ErrorKind::Verify => "verify",
            ErrorKind::Runtime => "runtime",
            ErrorKind::Type => "type",
            ErrorKind::Bounds => "bounds",
            ErrorKind::Host => "host",
        }
    }

    /// Whether this class of error is attributable to the input artifact (source
    /// or module) as opposed to the running program.
    pub fn is_static(self) -> bool {
        matches!(
            self,
            ErrorKind::Lex
                | ErrorKind::Parse
                | ErrorKind::Compile
                | ErrorKind::Decode
                | ErrorKind::Verify
        )
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A source position, used to attach location information to front-end errors.
///
/// Positions are 1-based for lines and columns because they are primarily shown
/// to humans. A `Span` with `line == 0` is a synthetic position with no source
/// origin (for example, an error raised while lowering compiler intrinsics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    /// Byte offset into the original source, useful for editors.
    pub offset: u32,
    /// Length in bytes of the spanned region.
    pub len: u32,
}

impl Span {
    pub const fn new(line: u32, col: u32, offset: u32, len: u32) -> Span {
        Span { line, col, offset, len }
    }

    /// A synthetic span with no source origin.
    pub const fn synthetic() -> Span {
        Span { line: 0, col: 0, offset: 0, len: 0 }
    }

    pub fn is_synthetic(&self) -> bool {
        self.line == 0
    }

    /// Merge two spans into the smallest span covering both. Synthetic spans are
    /// treated as absorbing, so merging with one yields the other.
    pub fn merge(self, other: Span) -> Span {
        if self.is_synthetic() {
            return other;
        }
        if other.is_synthetic() {
            return self;
        }
        let start = self.offset.min(other.offset);
        let end = (self.offset + self.len).max(other.offset + other.len);
        Span {
            line: self.line.min(other.line.max(1)),
            col: if self.offset <= other.offset { self.col } else { other.col },
            offset: start,
            len: end.saturating_sub(start),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_synthetic() {
            f.write_str("<synthetic>")
        } else {
            write!(f, "{}:{}", self.line, self.col)
        }
    }
}

/// The unified error type for the crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    span: Span,
}

impl Error {
    /// Construct an error with no source position.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into(), span: Span::synthetic() }
    }

    /// Construct an error located at `span`.
    pub fn at(kind: ErrorKind, span: Span, message: impl Into<String>) -> Error {
        Error { kind, message: message.into(), span }
    }

    /// Attach or replace the span on an existing error.
    pub fn with_span(mut self, span: Span) -> Error {
        self.span = span;
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> Span {
        self.span
    }

    // ---- Constructors used pervasively; grouped for readability. -----------

    pub fn lex(span: Span, message: impl Into<String>) -> Error {
        Error::at(ErrorKind::Lex, span, message)
    }

    pub fn parse(span: Span, message: impl Into<String>) -> Error {
        Error::at(ErrorKind::Parse, span, message)
    }

    pub fn compile(span: Span, message: impl Into<String>) -> Error {
        Error::at(ErrorKind::Compile, span, message)
    }

    pub fn decode(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Decode, message)
    }

    pub fn verify(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Verify, message)
    }

    pub fn runtime(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Runtime, message)
    }

    pub fn type_error(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Type, message)
    }

    pub fn bounds(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Bounds, message)
    }

    pub fn host(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Host, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.span.is_synthetic() {
            write!(f, "{}: {}", self.kind.tag(), self.message)
        } else {
            write!(f, "{}: {} (at {})", self.kind.tag(), self.message, self.span)
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_static_classification() {
        assert!(ErrorKind::Parse.is_static());
        assert!(!ErrorKind::Runtime.is_static());
        assert_eq!(ErrorKind::Bounds.tag(), "bounds");
    }

    #[test]
    fn span_merge_absorbs_synthetic() {
        let s = Span::new(3, 4, 10, 2);
        assert_eq!(s.merge(Span::synthetic()), s);
        assert_eq!(Span::synthetic().merge(s), s);
    }

    #[test]
    fn span_merge_covers_range() {
        let a = Span::new(1, 1, 0, 3);
        let b = Span::new(1, 6, 5, 2);
        let m = a.merge(b);
        assert_eq!(m.offset, 0);
        assert_eq!(m.len, 7);
    }

    #[test]
    fn error_display_includes_span() {
        let e = Error::parse(Span::new(2, 5, 8, 1), "unexpected token");
        let text = format!("{e}");
        assert!(text.contains("parse"));
        assert!(text.contains("2:5"));
    }
}
