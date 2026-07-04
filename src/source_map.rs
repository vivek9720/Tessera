//! Source file indexing and diagnostic rendering.
//!
//! The parser and compiler already carry byte-based [`crate::error::Span`]
//! values. This module turns those offsets back into line/column snippets for
//! editors, command-line diagnostics, and test fixtures. It is intentionally
//! independent of the lexer so tools can render diagnostics for partially
//! tokenized or even invalid source.

use std::fmt::Write as _;

use crate::error::{Error, ErrorKind, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: FileId,
    pub start: u32,
    pub len: u32,
}

impl SourceSpan {
    pub fn new(file: FileId, start: u32, len: u32) -> SourceSpan {
        SourceSpan { file, start, len }
    }

    pub fn end(self) -> u32 {
        self.start.saturating_add(self.len)
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    name: String,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0usize];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        SourceFile { name: name.into(), text, line_starts }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn byte_len(&self) -> usize {
        self.text.len()
    }

    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = (offset as usize).min(self.text.len());
        let idx = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        };
        let line_start = self.line_starts[idx];
        LineCol { line: idx as u32 + 1, col: offset.saturating_sub(line_start) as u32 + 1 }
    }

    pub fn offset_of(&self, line: u32, col: u32) -> Option<u32> {
        if line == 0 || col == 0 {
            return None;
        }
        let start = *self.line_starts.get(line as usize - 1)?;
        let end = self.line_end(line as usize - 1);
        let offset = start.saturating_add(col as usize - 1);
        if offset <= end {
            Some(offset as u32)
        } else {
            None
        }
    }

    pub fn line_text(&self, line: u32) -> Option<&str> {
        if line == 0 {
            return None;
        }
        let idx = line as usize - 1;
        let start = *self.line_starts.get(idx)?;
        let end = self.line_end(idx);
        self.text.get(start..end)
    }

    fn line_end(&self, idx: usize) -> usize {
        let raw_end = self
            .line_starts
            .get(idx + 1)
            .copied()
            .unwrap_or_else(|| self.text.len());
        let mut end = raw_end;
        if end > 0 && self.text.as_bytes()[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && self.text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end
    }

    pub fn span_text(&self, span: SourceSpan) -> Option<&str> {
        if span.file.0 != 0 {
            return None;
        }
        let start = span.start as usize;
        let end = span.end() as usize;
        self.text.get(start.min(self.text.len())..end.min(self.text.len()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap { files: Vec::new() }
    }

    pub fn add_file(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let id = FileId(self.files.len());
        self.files.push(SourceFile::new(name, text));
        id
    }

    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0)
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn span_from_error(&self, file: FileId, error: &Error) -> Option<SourceSpan> {
        let span = error.span();
        if span.is_synthetic() {
            None
        } else {
            Some(SourceSpan::new(file, span.offset, span.len.max(1)))
        }
    }

    pub fn render_error(&self, file: FileId, error: &Error) -> String {
        let severity = match error.kind() {
            ErrorKind::Lex | ErrorKind::Parse | ErrorKind::Compile => DiagnosticSeverity::Error,
            ErrorKind::Decode | ErrorKind::Verify => DiagnosticSeverity::Error,
            ErrorKind::Runtime | ErrorKind::Type | ErrorKind::Bounds | ErrorKind::Host => {
                DiagnosticSeverity::Error
            }
        };
        let mut diagnostic = Diagnostic::new(severity, error.message());
        if let Some(span) = self.span_from_error(file, error) {
            diagnostic = diagnostic.with_label(Label::primary(span, error.kind().tag()));
        }
        diagnostic.render(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Note,
    Warning,
    Error,
}

impl DiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticSeverity::Note => "note",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    pub span: SourceSpan,
    pub message: String,
    pub primary: bool,
}

impl Label {
    pub fn primary(span: SourceSpan, message: impl Into<String>) -> Label {
        Label { span, message: message.into(), primary: true }
    }

    pub fn secondary(span: SourceSpan, message: impl Into<String>) -> Label {
        Label { span, message: message.into(), primary: false }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    severity: DiagnosticSeverity,
    message: String,
    labels: Vec<Label>,
    notes: Vec<String>,
}

impl Diagnostic {
    pub fn new(severity: DiagnosticSeverity, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity, message: message.into(), labels: Vec::new(), notes: Vec::new() }
    }

    pub fn with_label(mut self, label: Label) -> Diagnostic {
        self.labels.push(label);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}: {}", self.severity.as_str(), self.message);
        let mut labels = self.labels.clone();
        labels.sort_by_key(|label| (label.span.file.0, label.span.start, !label.primary));
        for label in labels {
            render_label(&mut out, sources, &label);
        }
        for note in &self.notes {
            let _ = writeln!(out, "note: {note}");
        }
        out
    }
}

fn render_label(out: &mut String, sources: &SourceMap, label: &Label) {
    let Some(file) = sources.file(label.span.file) else {
        let _ = writeln!(out, " --> <unknown>:{}", label.span.start);
        return;
    };
    let lc = file.line_col(label.span.start);
    let _ = writeln!(out, " --> {}:{}:{}", file.name(), lc.line, lc.col);
    let Some(line) = file.line_text(lc.line) else {
        return;
    };
    let gutter = lc.line.to_string();
    let _ = writeln!(out, "{:>width$} |", "", width = gutter.len());
    let _ = writeln!(out, "{gutter} | {line}");
    let caret_col = lc.col.saturating_sub(1) as usize;
    let width = highlight_width(file, label.span, lc.line).max(1);
    let marker = if label.primary { '^' } else { '-' };
    let _ = write!(out, "{:>width$} | ", "", width = gutter.len());
    for _ in 0..caret_col {
        out.push(' ');
    }
    for _ in 0..width {
        out.push(marker);
    }
    if !label.message.is_empty() {
        let _ = write!(out, " {}", label.message);
    }
    out.push('\n');
}

fn highlight_width(file: &SourceFile, span: SourceSpan, line: u32) -> usize {
    let lc = file.line_col(span.start);
    if lc.line != line {
        return 1;
    }
    let line_start = file.offset_of(line, 1).unwrap_or(span.start);
    let line_len = file.line_text(line).map(|s| s.len()).unwrap_or(0) as u32;
    let line_end = line_start.saturating_add(line_len);
    let end = span.end().min(line_end);
    end.saturating_sub(span.start).max(1) as usize
}

pub fn annotate_error(file_name: &str, source: &str, error: &Error) -> String {
    let mut map = SourceMap::new();
    let id = map.add_file(file_name, source);
    map.render_error(id, error)
}

pub fn span_for_line(source: &str, line: u32) -> Option<Span> {
    let file = SourceFile::new("<memory>", source);
    let start = file.offset_of(line, 1)?;
    let len = file.line_text(line).map(|s| s.len()).unwrap_or(0) as u32;
    let lc = file.line_col(start);
    Some(Span::new(lc.line, lc.col, start, len))
}

pub fn line_index(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (idx, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push((idx + 1) as u32);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_tracks_offsets() {
        let file = SourceFile::new("demo.tsr", "let x = 1;\nreturn x;\n");
        assert_eq!(file.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(file.line_col(11), LineCol { line: 2, col: 1 });
        assert_eq!(file.offset_of(2, 8), Some(18));
    }

    #[test]
    fn renders_primary_label() {
        let mut map = SourceMap::new();
        let id = map.add_file("demo.tsr", "let x = ;\n");
        let span = SourceSpan::new(id, 8, 1);
        let text = Diagnostic::new(DiagnosticSeverity::Error, "expected expression")
            .with_label(Label::primary(span, "parse"))
            .render(&map);
        assert!(text.contains("demo.tsr:1:9"));
        assert!(text.contains("^ parse"));
    }

    #[test]
    fn span_for_line_uses_full_line() {
        let span = span_for_line("a\nsecond\n", 2).unwrap();
        assert_eq!(span.offset, 2);
        assert_eq!(span.len, 6);
    }
}
