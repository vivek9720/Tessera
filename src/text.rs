//! Small text and byte utilities shared by the CLI and formatting code.
//!
//! These helpers are pure and allocation-friendly: escaping strings for display,
//! producing a canonical hex dump of a byte buffer, wrapping text to a width,
//! and a couple of string predicates the surface language does not expose as
//! builtins. Keeping them in one place avoids re-implementing the same string
//! bookkeeping across the disassembler, the formatter, and the CLI.

/// Escape a string for single-line display, turning control characters into
/// their `\x`-style escapes and preserving printable UTF-8.
pub fn escape_display(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Produce a classic 16-byte-per-row hex dump with an ASCII gutter.
pub fn hexdump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let offset = row * 16;
        push_hex_row(&mut out, offset, chunk);
    }
    if bytes.is_empty() {
        out.push_str("00000000\n");
    }
    out
}

fn push_hex_row(out: &mut String, offset: usize, chunk: &[u8]) {
    use std::fmt::Write as _;
    let _ = write!(out, "{offset:08x}  ");
    for i in 0..16 {
        if i == 8 {
            out.push(' ');
        }
        match chunk.get(i) {
            Some(b) => {
                let _ = write!(out, "{b:02x} ");
            }
            None => out.push_str("   "),
        }
    }
    out.push_str(" |");
    for b in chunk {
        let c = *b;
        if (0x20..0x7f).contains(&c) {
            out.push(c as char);
        } else {
            out.push('.');
        }
    }
    out.push_str("|\n");
}

/// Wrap `text` to at most `width` columns on whitespace boundaries.
pub fn wrap(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let mut out = String::new();
    let mut col = 0usize;
    for (i, word) in text.split_whitespace().enumerate() {
        let wlen = word.chars().count();
        if i == 0 {
            out.push_str(word);
            col = wlen;
        } else if col + 1 + wlen > width {
            out.push('\n');
            out.push_str(word);
            col = wlen;
        } else {
            out.push(' ');
            out.push_str(word);
            col += 1 + wlen;
        }
    }
    out
}

/// Count occurrences of a (non-empty) substring, without overlaps.
pub fn count_substr(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut rest = haystack;
    while let Some(pos) = rest.find(needle) {
        count += 1;
        rest = &rest[pos + needle.len()..];
    }
    count
}

/// Split a string into fixed-size character groups, used for formatting numeric
/// output and identifiers in the disassembler.
pub fn chunk_chars(s: &str, size: usize) -> Vec<String> {
    if size == 0 {
        return vec![s.to_string()];
    }
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}

/// Whether `s` is a valid Tessera identifier (letters, digits, underscore, not
/// starting with a digit). Useful for tooling that decides whether to quote a
/// map key.
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_controls() {
        assert_eq!(escape_display("a\nb\t"), "a\\nb\\t");
        assert_eq!(escape_display("plain"), "plain");
    }

    #[test]
    fn hexdump_layout() {
        let dump = hexdump(b"ABC");
        assert!(dump.starts_with("00000000  41 42 43"));
        assert!(dump.contains("|ABC|"));
    }

    #[test]
    fn wrap_breaks_on_width() {
        let wrapped = wrap("one two three four", 8);
        assert!(wrapped.contains('\n'));
        for line in wrapped.lines() {
            assert!(line.chars().count() <= 8 || !line.contains(' '));
        }
    }

    #[test]
    fn substr_count() {
        assert_eq!(count_substr("aaaa", "aa"), 2);
        assert_eq!(count_substr("abcabc", "bc"), 2);
        assert_eq!(count_substr("x", ""), 0);
    }

    #[test]
    fn identifier_predicate() {
        assert!(is_identifier("foo_bar1"));
        assert!(!is_identifier("1abc"));
        assert!(!is_identifier("has space"));
        assert!(!is_identifier(""));
    }
}
