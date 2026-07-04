//! The runtime value representation.
//!
//! [`Value`] is a small `Copy` tagged union. Scalars (nil, booleans, 64-bit
//! integers, doubles) live inline; everything with identity or variable size
//! (strings, lists, maps, closures, upvalue cells) lives on the [`crate::heap`]
//! and is referenced through a [`Handle`]. Because `Value` is `Copy`, register
//! buffers and the operand stack are plain arrays of values, which keeps the
//! interpreter's hot path free of reference-count traffic.

use std::fmt;

/// An index into the object heap. The tag byte distinguishing object kinds is
/// stored with the object itself, not in the handle, so handles are opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u32);

impl Handle {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

/// A runtime value.
#[derive(Debug, Clone, Copy)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Obj(Handle),
}

/// A coarse type tag, used for error messages and the `type` builtin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Nil,
    Bool,
    Int,
    Float,
    Object,
}

impl ValueType {
    pub fn name(self) -> &'static str {
        match self {
            ValueType::Nil => "nil",
            ValueType::Bool => "bool",
            ValueType::Int => "int",
            ValueType::Float => "float",
            ValueType::Object => "object",
        }
    }
}

impl Value {
    pub const NIL: Value = Value::Nil;
    pub const TRUE: Value = Value::Bool(true);
    pub const FALSE: Value = Value::Bool(false);

    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Nil => ValueType::Nil,
            Value::Bool(_) => ValueType::Bool,
            Value::Int(_) => ValueType::Int,
            Value::Float(_) => ValueType::Float,
            Value::Obj(_) => ValueType::Object,
        }
    }

    /// Tessera truthiness: `nil` and `false` are falsey; every other value,
    /// including `0` and the empty string, is truthy.
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub fn as_handle(&self) -> Option<Handle> {
        match self {
            Value::Obj(h) => Some(*h),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Coerce a numeric value to `f64`. Integers convert lossily for very large
    /// magnitudes, matching the behaviour of the arithmetic opcodes.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(x) => Some(*x),
            _ => None,
        }
    }

    /// Scalar equality that ignores object identity. Object handles compare
    /// equal only when they are the same handle; structural equality of objects
    /// is handled by the heap, which has access to the payloads.
    pub fn scalar_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                (*a as f64) == *b
            }
            (Value::Obj(a), Value::Obj(b)) => a == b,
            _ => false,
        }
    }
}

impl Default for Value {
    fn default() -> Value {
        Value::Nil
    }
}

/// A shallow textual form used in disassembly and low-level diagnostics. The
/// user-facing `to_string` builtin renders objects through the heap; this is
/// only for scalars and bare handles.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => f.write_str("nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => {
                if x.fract() == 0.0 && x.is_finite() {
                    write!(f, "{x:.1}")
                } else {
                    write!(f, "{x}")
                }
            }
            Value::Obj(h) => write!(f, "{h}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness_rules() {
        assert!(!Value::Nil.is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(Value::Int(0).is_truthy());
        assert!(Value::Float(0.0).is_truthy());
    }

    #[test]
    fn scalar_equality_mixes_numbers() {
        assert!(Value::Int(3).scalar_eq(&Value::Float(3.0)));
        assert!(!Value::Int(3).scalar_eq(&Value::Float(3.5)));
        assert!(Value::Obj(Handle(1)).scalar_eq(&Value::Obj(Handle(1))));
        assert!(!Value::Obj(Handle(1)).scalar_eq(&Value::Obj(Handle(2))));
    }

    #[test]
    fn number_coercion() {
        assert_eq!(Value::Int(2).as_number(), Some(2.0));
        assert_eq!(Value::Float(2.5).as_number(), Some(2.5));
        assert_eq!(Value::Nil.as_number(), None);
    }
}
