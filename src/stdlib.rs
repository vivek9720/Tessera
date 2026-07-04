//! Machine-readable standard-library reference.
//!
//! The runtime table in [`crate::builtins`] maps names to native ids. This module
//! describes those builtins for help output, documentation generators, and
//! language-server integrations. Keeping the reference in code lets tests check
//! that the documented names stay synchronized with the runtime registration
//! table.

use crate::builtins;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeName {
    Any,
    Nil,
    Bool,
    Int,
    Float,
    Number,
    String,
    Bytes,
    List,
    Map,
}

impl TypeName {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeName::Any => "any",
            TypeName::Nil => "nil",
            TypeName::Bool => "bool",
            TypeName::Int => "int",
            TypeName::Float => "float",
            TypeName::Number => "number",
            TypeName::String => "str",
            TypeName::Bytes => "bytes",
            TypeName::List => "list",
            TypeName::Map => "map",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterDoc {
    pub name: &'static str,
    pub ty: TypeName,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinDoc {
    pub name: &'static str,
    pub returns: TypeName,
    pub params: &'static [ParameterDoc],
    pub summary: &'static str,
    pub details: &'static str,
}

macro_rules! p {
    ($name:literal : $ty:ident) => {
        ParameterDoc { name: $name, ty: TypeName::$ty, optional: false }
    };
    ($name:literal : $ty:ident ?) => {
        ParameterDoc { name: $name, ty: TypeName::$ty, optional: true }
    };
}

const PRINT_PARAMS: &[ParameterDoc] = &[p!("value": Any ?)];
const LEN_PARAMS: &[ParameterDoc] = &[p!("value": Any)];
const CONVERT_PARAMS: &[ParameterDoc] = &[p!("value": Any)];
const NUM_PARAMS: &[ParameterDoc] = &[p!("value": Number)];
const MINMAX_PARAMS: &[ParameterDoc] = &[p!("first": Number), p!("rest": Number ?)];
const ORD_PARAMS: &[ParameterDoc] = &[p!("text": String)];
const CHR_PARAMS: &[ParameterDoc] = &[p!("code": Int)];
const SUBSTR_PARAMS: &[ParameterDoc] = &[p!("text": String), p!("start": Int), p!("end": Int ?)];
const LIST_VALUE_PARAMS: &[ParameterDoc] = &[p!("list": List), p!("value": Any)];
const LIST_PARAMS: &[ParameterDoc] = &[p!("list": List)];
const CONTAINS_PARAMS: &[ParameterDoc] = &[p!("container": Any), p!("value": Any)];
const RANGE_PARAMS: &[ParameterDoc] = &[p!("start_or_end": Int), p!("end": Int ?), p!("step": Int ?)];
const ASSERT_PARAMS: &[ParameterDoc] = &[p!("condition": Any), p!("message": Any ?)];
const REPEAT_PARAMS: &[ParameterDoc] = &[p!("text": String), p!("count": Int)];
const JOIN_PARAMS: &[ParameterDoc] = &[p!("list": List), p!("separator": String ?)];
const CLAMP_PARAMS: &[ParameterDoc] = &[p!("value": Number), p!("min": Number), p!("max": Number)];
const IS_NIL_PARAMS: &[ParameterDoc] = &[p!("value": Any ?)];

pub const BUILTIN_DOCS: &[BuiltinDoc] = &[
    BuiltinDoc {
        name: "print",
        returns: TypeName::Nil,
        params: PRINT_PARAMS,
        summary: "write values to the VM output buffer without a trailing newline",
        details: "Values are rendered with the same display rules used by the command-line runner.",
    },
    BuiltinDoc {
        name: "println",
        returns: TypeName::Nil,
        params: PRINT_PARAMS,
        summary: "write values to the VM output buffer followed by a newline",
        details: "Arguments are separated with a single space before the newline is appended.",
    },
    BuiltinDoc {
        name: "len",
        returns: TypeName::Int,
        params: LEN_PARAMS,
        summary: "return the length of a string, byte blob, list, or map",
        details: "Scalar values have no length and produce a runtime type error.",
    },
    BuiltinDoc { name: "type", returns: TypeName::String, params: CONVERT_PARAMS, summary: "return the runtime type name of a value", details: "Object values report their heap object kind." },
    BuiltinDoc { name: "str", returns: TypeName::String, params: CONVERT_PARAMS, summary: "convert a value to display text", details: "Useful when building logs or serialized diagnostics in scripts." },
    BuiltinDoc { name: "int", returns: TypeName::Int, params: CONVERT_PARAMS, summary: "convert a number, boolean, or numeric string to an integer", details: "Floating point values truncate toward zero." },
    BuiltinDoc { name: "float", returns: TypeName::Float, params: CONVERT_PARAMS, summary: "convert an integer or numeric string to floating point", details: "Invalid strings are reported as runtime errors." },
    BuiltinDoc { name: "bool", returns: TypeName::Bool, params: CONVERT_PARAMS, summary: "return the truthiness of a value", details: "Nil and false are falsey; all other values are truthy." },
    BuiltinDoc { name: "abs", returns: TypeName::Number, params: NUM_PARAMS, summary: "absolute value", details: "Integer absolute value uses wrapping semantics for the minimum integer." },
    BuiltinDoc { name: "min", returns: TypeName::Number, params: MINMAX_PARAMS, summary: "smallest numeric argument", details: "At least one argument is required." },
    BuiltinDoc { name: "max", returns: TypeName::Number, params: MINMAX_PARAMS, summary: "largest numeric argument", details: "At least one argument is required." },
    BuiltinDoc { name: "floor", returns: TypeName::Int, params: NUM_PARAMS, summary: "round a number down", details: "The result is represented as an integer." },
    BuiltinDoc { name: "ceil", returns: TypeName::Int, params: NUM_PARAMS, summary: "round a number up", details: "The result is represented as an integer." },
    BuiltinDoc { name: "sqrt", returns: TypeName::Float, params: NUM_PARAMS, summary: "square root", details: "Negative inputs produce a runtime error rather than NaN." },
    BuiltinDoc { name: "ord", returns: TypeName::Int, params: ORD_PARAMS, summary: "Unicode scalar value of the first character", details: "The input string must not be empty." },
    BuiltinDoc { name: "chr", returns: TypeName::String, params: CHR_PARAMS, summary: "string containing one Unicode scalar value", details: "Invalid code points produce a runtime error." },
    BuiltinDoc { name: "substr", returns: TypeName::String, params: SUBSTR_PARAMS, summary: "substring by character index", details: "The optional end index is clamped to the string length." },
    BuiltinDoc { name: "upper", returns: TypeName::String, params: ORD_PARAMS, summary: "uppercase conversion", details: "Uses Rust's Unicode-aware case conversion." },
    BuiltinDoc { name: "lower", returns: TypeName::String, params: ORD_PARAMS, summary: "lowercase conversion", details: "Uses Rust's Unicode-aware case conversion." },
    BuiltinDoc { name: "push", returns: TypeName::Int, params: LIST_VALUE_PARAMS, summary: "append a value to a list", details: "Returns the new list length." },
    BuiltinDoc { name: "pop", returns: TypeName::Any, params: LIST_PARAMS, summary: "remove and return the last list item", details: "Returns nil when the list is empty." },
    BuiltinDoc { name: "keys", returns: TypeName::List, params: LIST_PARAMS, summary: "return map keys", details: "The result order follows the map's insertion order." },
    BuiltinDoc { name: "values", returns: TypeName::List, params: LIST_PARAMS, summary: "return map values", details: "The result order follows the map's insertion order." },
    BuiltinDoc { name: "contains", returns: TypeName::Bool, params: CONTAINS_PARAMS, summary: "membership test for lists, maps, and strings", details: "Maps are searched by key; strings require a string needle." },
    BuiltinDoc { name: "range", returns: TypeName::List, params: RANGE_PARAMS, summary: "build a half-open integer range", details: "The step must not be zero and the result is limited by the collection cap." },
    BuiltinDoc { name: "assert", returns: TypeName::Any, params: ASSERT_PARAMS, summary: "raise a runtime error when a condition is falsey", details: "Returns the condition value when it succeeds." },
    BuiltinDoc { name: "repeat", returns: TypeName::String, params: REPEAT_PARAMS, summary: "repeat a string", details: "The result is checked against the maximum string length." },
    BuiltinDoc { name: "join", returns: TypeName::String, params: JOIN_PARAMS, summary: "join list items into a string", details: "Each item is formatted with the normal display renderer." },
    BuiltinDoc { name: "clamp", returns: TypeName::Float, params: CLAMP_PARAMS, summary: "clamp a number to an inclusive range", details: "The lower bound must not exceed the upper bound." },
    BuiltinDoc { name: "sign", returns: TypeName::Int, params: NUM_PARAMS, summary: "return -1, 0, or 1 for a number", details: "Floating point values compare against zero." },
    BuiltinDoc { name: "is_nil", returns: TypeName::Bool, params: IS_NIL_PARAMS, summary: "test whether a value is nil", details: "Missing arguments are treated as nil." },
];

pub fn builtin_doc(name: &str) -> Option<&'static BuiltinDoc> {
    BUILTIN_DOCS.iter().find(|doc| doc.name == name)
}

pub fn render_builtin(name: &str) -> Option<String> {
    builtin_doc(name).map(render_doc)
}

pub fn render_all() -> String {
    let mut out = String::new();
    for doc in BUILTIN_DOCS {
        out.push_str(&render_doc(doc));
        out.push('\n');
    }
    out
}

fn render_doc(doc: &BuiltinDoc) -> String {
    let mut out = String::new();
    out.push_str(doc.name);
    out.push('(');
    for (idx, param) in doc.params.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(param.name);
        out.push_str(": ");
        out.push_str(param.ty.as_str());
        if param.optional {
            out.push('?');
        }
    }
    out.push_str(") -> ");
    out.push_str(doc.returns.as_str());
    out.push('\n');
    out.push_str("  ");
    out.push_str(doc.summary);
    out.push('\n');
    out.push_str("  ");
    out.push_str(doc.details);
    out
}

pub fn docs_are_synchronized() -> bool {
    builtins::BUILTINS.len() == BUILTIN_DOCS.len()
        && builtins::BUILTINS
            .iter()
            .all(|(name, _)| BUILTIN_DOCS.iter().any(|doc| doc.name == *name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_docs_match_runtime_names() {
        assert!(docs_are_synchronized());
    }

    #[test]
    fn render_single_builtin() {
        let text = render_builtin("range").unwrap();
        assert!(text.contains("range("));
        assert!(text.contains("list"));
    }
}
