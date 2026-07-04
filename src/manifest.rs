//! Tessera project manifest support.
//!
//! A `.tessera` project can contain a `Tessera.toml` file describing the
//! package, entry points, scripts, and local dependencies. This parser accepts a
//! deliberately small TOML subset: section headers, `key = value` assignments,
//! quoted strings, booleans, string arrays, and inline dependency tables. It is
//! enough for build tools without pulling a TOML crate into the embeddable core.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, ErrorKind, Result, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectManifest {
    pub package: PackageInfo,
    pub entries: Vec<EntryPoint>,
    pub dependencies: Vec<Dependency>,
    pub scripts: BTreeMap<String, String>,
    pub features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: Version,
    pub edition: String,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub description: Option<String>,
    pub publish: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPoint {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Binary,
    Library,
    Test,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub requirement: DependencyReq,
    pub optional: bool,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyReq {
    Version(VersionReq),
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    Exact(Version),
    Caret(Version),
    GreaterEqual(Version),
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Section {
    Package,
    Dependencies,
    Scripts,
    Features,
    Entry(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    String(String),
    Bool(bool),
    Array(Vec<String>),
    Inline(BTreeMap<String, Value>),
}

impl Default for ProjectManifest {
    fn default() -> Self {
        ProjectManifest {
            package: PackageInfo {
                name: String::new(),
                version: Version { major: 0, minor: 1, patch: 0 },
                edition: "2021".to_string(),
                authors: Vec::new(),
                license: None,
                description: None,
                publish: false,
            },
            entries: Vec::new(),
            dependencies: Vec::new(),
            scripts: BTreeMap::new(),
            features: BTreeMap::new(),
        }
    }
}

impl Version {
    pub fn parse(input: &str) -> Result<Version> {
        let mut parts = input.trim().split('.');
        let major = parse_part(parts.next(), "major")?;
        let minor = parse_part(parts.next(), "minor")?;
        let patch = parse_part(parts.next(), "patch")?;
        if parts.next().is_some() {
            return Err(Error::new(ErrorKind::Parse, "version has too many components"));
        }
        Ok(Version { major, minor, patch })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl VersionReq {
    pub fn parse(input: &str) -> Result<VersionReq> {
        let input = input.trim();
        if input == "*" {
            return Ok(VersionReq::Any);
        }
        if let Some(rest) = input.strip_prefix('^') {
            return Ok(VersionReq::Caret(Version::parse(rest)?));
        }
        if let Some(rest) = input.strip_prefix(">=") {
            return Ok(VersionReq::GreaterEqual(Version::parse(rest)?));
        }
        Ok(VersionReq::Exact(Version::parse(input)?))
    }

    pub fn matches(&self, version: Version) -> bool {
        match self {
            VersionReq::Any => true,
            VersionReq::Exact(want) => version == *want,
            VersionReq::GreaterEqual(min) => version >= *min,
            VersionReq::Caret(base) => {
                version.major == base.major && version >= *base
            }
        }
    }
}

impl EntryKind {
    pub fn parse(input: &str) -> EntryKind {
        match input.trim() {
            "lib" | "library" => EntryKind::Library,
            "test" => EntryKind::Test,
            "tool" => EntryKind::Tool,
            _ => EntryKind::Binary,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Binary => "bin",
            EntryKind::Library => "lib",
            EntryKind::Test => "test",
            EntryKind::Tool => "tool",
        }
    }
}

pub fn parse_manifest(src: &str) -> Result<ProjectManifest> {
    let mut parser = ManifestParser::new(src);
    parser.parse()
}

struct ManifestParser<'a> {
    src: &'a str,
    section: Section,
    manifest: ProjectManifest,
    seen_entries: BTreeSet<String>,
}

impl<'a> ManifestParser<'a> {
    fn new(src: &'a str) -> ManifestParser<'a> {
        ManifestParser {
            src,
            section: Section::Package,
            manifest: ProjectManifest::default(),
            seen_entries: BTreeSet::new(),
        }
    }

    fn parse(&mut self) -> Result<ProjectManifest> {
        for (line_idx, raw) in self.src.lines().enumerate() {
            let line_no = line_idx as u32 + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                self.section = parse_section(line, line_no)?;
                continue;
            }
            let (key, value) = parse_assignment(line, line_no)?;
            self.apply_value(key, value, line_no)?;
        }
        self.finish()
    }

    fn apply_value(&mut self, key: String, value: Value, line_no: u32) -> Result<()> {
        let section = self.section.clone();
        match section {
            Section::Package => self.apply_package(key, value, line_no),
            Section::Dependencies => self.apply_dependency(key, value, line_no),
            Section::Scripts => {
                self.manifest.scripts.insert(key, expect_string(value, line_no)?);
                Ok(())
            }
            Section::Features => {
                self.manifest.features.insert(key, expect_array(value, line_no)?);
                Ok(())
            }
            Section::Entry(name) => self.apply_entry(name, key, value, line_no),
        }
    }

    fn apply_package(&mut self, key: String, value: Value, line_no: u32) -> Result<()> {
        match key.as_str() {
            "name" => self.manifest.package.name = expect_string(value, line_no)?,
            "version" => self.manifest.package.version = Version::parse(&expect_string(value, line_no)?)?,
            "edition" => self.manifest.package.edition = expect_string(value, line_no)?,
            "authors" => self.manifest.package.authors = expect_array(value, line_no)?,
            "license" => self.manifest.package.license = Some(expect_string(value, line_no)?),
            "description" => self.manifest.package.description = Some(expect_string(value, line_no)?),
            "publish" => self.manifest.package.publish = expect_bool(value, line_no)?,
            other => return Err(parse_error(line_no, format!("unknown package key `{other}`"))),
        }
        Ok(())
    }

    fn apply_dependency(&mut self, name: String, value: Value, line_no: u32) -> Result<()> {
        let dependency = match value {
            Value::String(req) => Dependency {
                name,
                requirement: DependencyReq::Version(VersionReq::parse(&req)?),
                optional: false,
                features: Vec::new(),
            },
            Value::Inline(mut table) => {
                let optional = table
                    .remove("optional")
                    .map(|value| expect_bool(value, line_no))
                    .transpose()?
                    .unwrap_or(false);
                let features = table
                    .remove("features")
                    .map(|value| expect_array(value, line_no))
                    .transpose()?
                    .unwrap_or_default();
                let requirement = if let Some(path) = table.remove("path") {
                    DependencyReq::Path(expect_string(path, line_no)?)
                } else if let Some(version) = table.remove("version") {
                    DependencyReq::Version(VersionReq::parse(&expect_string(version, line_no)?)?)
                } else {
                    DependencyReq::Version(VersionReq::Any)
                };
                Dependency { name, requirement, optional, features }
            }
            _ => return Err(parse_error(line_no, "dependency must be a string or inline table")),
        };
        self.manifest.dependencies.push(dependency);
        Ok(())
    }

    fn apply_entry(&mut self, name: String, key: String, value: Value, line_no: u32) -> Result<()> {
        let idx = match self.manifest.entries.iter().position(|entry| entry.name == name) {
            Some(idx) => idx,
            None => {
                if !self.seen_entries.insert(name.clone()) {
                    return Err(parse_error(line_no, format!("duplicate entry `{name}`")));
                }
                self.manifest.entries.push(EntryPoint {
                    name: name.clone(),
                    path: String::new(),
                    kind: EntryKind::Binary,
                });
                self.manifest.entries.len() - 1
            }
        };
        let entry = &mut self.manifest.entries[idx];
        match key.as_str() {
            "path" => entry.path = expect_string(value, line_no)?,
            "kind" => entry.kind = EntryKind::parse(&expect_string(value, line_no)?),
            other => return Err(parse_error(line_no, format!("unknown entry key `{other}`"))),
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<ProjectManifest> {
        validate_name(&self.manifest.package.name)?;
        if self.manifest.entries.is_empty() {
            self.manifest.entries.push(EntryPoint {
                name: self.manifest.package.name.clone(),
                path: "src/main.tsr".to_string(),
                kind: EntryKind::Binary,
            });
        }
        for entry in &self.manifest.entries {
            validate_entry(entry)?;
        }
        Ok(std::mem::take(&mut self.manifest))
    }
}

fn parse_section(line: &str, line_no: u32) -> Result<Section> {
    if !line.ends_with(']') {
        return Err(parse_error(line_no, "unterminated section header"));
    }
    let name = &line[1..line.len() - 1];
    match name {
        "package" => Ok(Section::Package),
        "dependencies" => Ok(Section::Dependencies),
        "scripts" => Ok(Section::Scripts),
        "features" => Ok(Section::Features),
        name if name.starts_with("entry.") => {
            let entry = name.trim_start_matches("entry.").trim_matches('"');
            validate_name(entry)?;
            Ok(Section::Entry(entry.to_string()))
        }
        other => Err(parse_error(line_no, format!("unknown manifest section `{other}`"))),
    }
}

fn parse_assignment(line: &str, line_no: u32) -> Result<(String, Value)> {
    let Some((key, value)) = split_once_outside_quotes(line, '=') else {
        return Err(parse_error(line_no, "expected `key = value` assignment"));
    };
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(parse_error(line_no, "empty manifest key"));
    }
    Ok((key, parse_value(value.trim(), line_no)?))
}

fn parse_value(input: &str, line_no: u32) -> Result<Value> {
    let input = input.trim();
    if input.starts_with('"') {
        return Ok(Value::String(parse_quoted(input, line_no)?));
    }
    if input.starts_with('[') {
        return parse_array(input, line_no);
    }
    if input.starts_with('{') {
        return parse_inline_table(input, line_no);
    }
    match input {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => Err(parse_error(line_no, "manifest value must be quoted, boolean, array, or inline table")),
    }
}

fn parse_array(input: &str, line_no: u32) -> Result<Value> {
    if !input.ends_with(']') {
        return Err(parse_error(line_no, "unterminated array"));
    }
    let body = &input[1..input.len() - 1];
    let mut values = Vec::new();
    for part in split_top_level(body, ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        values.push(parse_quoted(part, line_no)?);
    }
    Ok(Value::Array(values))
}

fn parse_inline_table(input: &str, line_no: u32) -> Result<Value> {
    if !input.ends_with('}') {
        return Err(parse_error(line_no, "unterminated inline table"));
    }
    let body = &input[1..input.len() - 1];
    let mut values = BTreeMap::new();
    for part in split_top_level(body, ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = split_once_outside_quotes(part, '=') else {
            return Err(parse_error(line_no, "expected key/value in inline table"));
        };
        values.insert(key.trim().to_string(), parse_value(value.trim(), line_no)?);
    }
    Ok(Value::Inline(values))
}

fn parse_quoted(input: &str, line_no: u32) -> Result<String> {
    if !input.starts_with('"') || !input.ends_with('"') {
        return Err(parse_error(line_no, "expected quoted string"));
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in input[1..input.len() - 1].chars() {
        if escaped {
            match ch {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                other => out.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        return Err(parse_error(line_no, "dangling escape in string"));
    }
    Ok(out)
}

fn strip_comment(line: &str) -> &str {
    let mut escaped = false;
    let mut quoted = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn split_once_outside_quotes(input: &str, needle: char) -> Option<(&str, &str)> {
    let mut escaped = false;
    let mut quoted = false;
    let mut depth = 0i32;
    for (idx, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            ch if ch == needle && !quoted && depth == 0 => {
                return Some((&input[..idx], &input[idx + ch.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(input: &str, needle: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut escaped = false;
    let mut quoted = false;
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            ch if ch == needle && !quoted && depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

fn expect_string(value: Value, line_no: u32) -> Result<String> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err(parse_error(line_no, "expected string value")),
    }
}

fn expect_bool(value: Value, line_no: u32) -> Result<bool> {
    match value {
        Value::Bool(value) => Ok(value),
        _ => Err(parse_error(line_no, "expected boolean value")),
    }
}

fn expect_array(value: Value, line_no: u32) -> Result<Vec<String>> {
    match value {
        Value::Array(value) => Ok(value),
        _ => Err(parse_error(line_no, "expected string array")),
    }
}

fn parse_part(part: Option<&str>, name: &str) -> Result<u16> {
    let part = part.ok_or_else(|| Error::new(ErrorKind::Parse, format!("version missing {name} component")))?;
    part.parse::<u16>()
        .map_err(|_| Error::new(ErrorKind::Parse, format!("invalid {name} version component")))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::new(ErrorKind::Parse, "package or entry name is empty"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::new(
            ErrorKind::Parse,
            format!("invalid package or entry name `{name}`"),
        ));
    }
    Ok(())
}

fn validate_entry(entry: &EntryPoint) -> Result<()> {
    validate_name(&entry.name)?;
    if entry.path.is_empty() {
        return Err(Error::new(
            ErrorKind::Parse,
            format!("entry `{}` has no path", entry.name),
        ));
    }
    if entry.path.contains("..") || entry.path.starts_with('/') || entry.path.starts_with('\\') {
        return Err(Error::new(
            ErrorKind::Parse,
            format!("entry `{}` path must be project-relative", entry.name),
        ));
    }
    Ok(())
}

fn parse_error(line_no: u32, message: impl Into<String>) -> Error {
    Error::parse(Span::new(line_no, 1, 0, 1), message)
}

pub fn manifest_summary(manifest: &ProjectManifest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} ({})\n",
        manifest.package.name, manifest.package.version, manifest.package.edition
    ));
    if let Some(description) = &manifest.package.description {
        out.push_str(description);
        out.push('\n');
    }
    out.push_str("entries:\n");
    for entry in &manifest.entries {
        out.push_str(&format!(
            "  {} [{}] {}\n",
            entry.name,
            entry.kind.as_str(),
            entry.path
        ));
    }
    if !manifest.dependencies.is_empty() {
        out.push_str("dependencies:\n");
        for dep in &manifest.dependencies {
            out.push_str(&format!("  {}\n", dependency_display(dep)));
        }
    }
    out
}

fn dependency_display(dep: &Dependency) -> String {
    let req = match &dep.requirement {
        DependencyReq::Path(path) => format!("path={path}"),
        DependencyReq::Version(VersionReq::Any) => "*".to_string(),
        DependencyReq::Version(VersionReq::Exact(v)) => v.to_string(),
        DependencyReq::Version(VersionReq::Caret(v)) => format!("^{v}"),
        DependencyReq::Version(VersionReq::GreaterEqual(v)) => format!(">={v}"),
    };
    if dep.optional {
        format!("{} ({req}, optional)", dep.name)
    } else {
        format!("{} ({req})", dep.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_manifest() {
        let src = r#"
            [package]
            name = "demo"
            version = "1.2.3"
            authors = ["A. User"]

            [entry.cli]
            path = "src/main.tsr"
            kind = "bin"

            [dependencies]
            math = "^1.0.0"
            local = { path = "vendor/local", optional = true, features = ["fast"] }
        "#;
        let manifest = parse_manifest(src).unwrap();
        assert_eq!(manifest.package.name, "demo");
        assert_eq!(manifest.entries[0].name, "cli");
        assert_eq!(manifest.dependencies.len(), 2);
    }

    #[test]
    fn rejects_absolute_entry_path() {
        let src = r#"
            [package]
            name = "demo"
            [entry.bad]
            path = "/tmp/main.tsr"
        "#;
        assert!(parse_manifest(src).is_err());
    }

    #[test]
    fn version_requirements_match() {
        let req = VersionReq::parse("^2.1.0").unwrap();
        assert!(req.matches(Version::parse("2.2.0").unwrap()));
        assert!(!req.matches(Version::parse("3.0.0").unwrap()));
    }
}
