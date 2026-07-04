//! The native standard library.
//!
//! Builtins are ordinary global functions backed by Rust. Each is registered on
//! a fresh [`crate::vm::Vm`] as an [`crate::heap::Object::Native`] handle bound
//! to a global name; the interpreter dispatches a call on a native object to
//! [`dispatch`]. Builtins operate on the heap and a shared output buffer and
//! return a [`Value`]; argument-count and type mismatches surface as ordinary
//! [`Error`]s rather than panics.

use crate::error::{Error, Result};
use crate::heap::{Heap, Object};
use crate::limits::MAX_STRING_LEN;
use crate::value::{Value, ValueType};

/// Name/id pairs registered into a VM's global environment at startup. The id is
/// the discriminator [`dispatch`] switches on.
pub const BUILTINS: &[(&str, u32)] = &[
    ("print", 0),
    ("println", 1),
    ("len", 2),
    ("type", 3),
    ("str", 4),
    ("int", 5),
    ("float", 6),
    ("bool", 7),
    ("abs", 8),
    ("min", 9),
    ("max", 10),
    ("floor", 11),
    ("ceil", 12),
    ("sqrt", 13),
    ("ord", 14),
    ("chr", 15),
    ("substr", 16),
    ("upper", 17),
    ("lower", 18),
    ("push", 19),
    ("pop", 20),
    ("keys", 21),
    ("values", 22),
    ("contains", 23),
    ("range", 24),
    ("assert", 25),
    ("repeat", 26),
    ("join", 27),
    ("clamp", 28),
    ("sign", 29),
    ("is_nil", 30),
];

/// The highest builtin id, used for a compile-time sanity check in the VM.
pub const BUILTIN_COUNT: u32 = 31;

/// Dispatch a native call.
pub fn dispatch(id: u32, heap: &mut Heap, args: &[Value], out: &mut String) -> Result<Value> {
    match id {
        0 => builtin_print(heap, args, out, false),
        1 => builtin_print(heap, args, out, true),
        2 => builtin_len(heap, args),
        3 => builtin_type(heap, args),
        4 => builtin_to_str(heap, args),
        5 => builtin_to_int(heap, args),
        6 => builtin_to_float(heap, args),
        7 => builtin_to_bool(args),
        8 => builtin_abs(args),
        9 => builtin_min(heap, args),
        10 => builtin_max(heap, args),
        11 => builtin_floor(args),
        12 => builtin_ceil(args),
        13 => builtin_sqrt(args),
        14 => builtin_ord(heap, args),
        15 => builtin_chr(heap, args),
        16 => builtin_substr(heap, args),
        17 => builtin_case(heap, args, true),
        18 => builtin_case(heap, args, false),
        19 => builtin_push(heap, args),
        20 => builtin_pop(heap, args),
        21 => builtin_keys(heap, args),
        22 => builtin_values(heap, args),
        23 => builtin_contains(heap, args),
        24 => builtin_range(heap, args),
        25 => builtin_assert(heap, args),
        26 => builtin_repeat(heap, args),
        27 => builtin_join(heap, args),
        28 => builtin_clamp(args),
        29 => builtin_sign(args),
        30 => Ok(Value::Bool(args.first().map_or(true, |v| v.is_nil()))),
        other => Err(Error::host(format!("unknown native function id {other}"))),
    }
}

// ---- Argument helpers ------------------------------------------------------

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Nil)
}

fn need_args(name: &str, args: &[Value], n: usize) -> Result<()> {
    if args.len() < n {
        return Err(Error::host(format!(
            "{name} expects {n} argument(s), got {}",
            args.len()
        )));
    }
    Ok(())
}

fn as_int(v: Value, ctx: &str) -> Result<i64> {
    match v {
        Value::Int(i) => Ok(i),
        Value::Float(f) => Ok(f as i64),
        Value::Bool(b) => Ok(b as i64),
        _ => Err(Error::type_error(format!("{ctx}: expected a number"))),
    }
}

fn as_number(v: Value, ctx: &str) -> Result<f64> {
    v.as_number()
        .ok_or_else(|| Error::type_error(format!("{ctx}: expected a number")))
}

// ---- Builtins --------------------------------------------------------------

fn builtin_print(heap: &mut Heap, args: &[Value], out: &mut String, newline: bool) -> Result<Value> {
    let mut first = true;
    for a in args {
        if !first {
            out.push(' ');
        }
        first = false;
        let rendered = heap.display(*a, 8);
        if out.len() + rendered.len() > MAX_STRING_LEN {
            return Err(Error::runtime("output buffer limit exceeded"));
        }
        out.push_str(&rendered);
    }
    if newline {
        out.push('\n');
    }
    Ok(Value::Nil)
}

fn builtin_len(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("len", args, 1)?;
    match arg(args, 0) {
        Value::Obj(h) => Ok(Value::Int(heap.length_of(h)?)),
        other => Err(Error::type_error(format!(
            "len: {} has no length",
            other.value_type().name()
        ))),
    }
}

fn builtin_type(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let name = match arg(args, 0) {
        Value::Obj(h) => heap.get(h)?.kind_name(),
        other => other.value_type().name(),
    };
    let handle = heap.new_string(name)?;
    Ok(Value::Obj(handle))
}

fn builtin_to_str(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let s = heap.display(arg(args, 0), 8);
    Ok(Value::Obj(heap.new_string(s)?))
}

fn builtin_to_int(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    match arg(args, 0) {
        Value::Int(i) => Ok(Value::Int(i)),
        Value::Float(f) => Ok(Value::Int(f as i64)),
        Value::Bool(b) => Ok(Value::Int(b as i64)),
        Value::Obj(h) => {
            let s = heap.as_str(h)?;
            s.trim()
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| Error::runtime("int: string is not an integer"))
        }
        Value::Nil => Err(Error::type_error("int: cannot convert nil")),
    }
}

fn builtin_to_float(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    match arg(args, 0) {
        Value::Int(i) => Ok(Value::Float(i as f64)),
        Value::Float(f) => Ok(Value::Float(f)),
        Value::Obj(h) => {
            let s = heap.as_str(h)?;
            s.trim()
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|_| Error::runtime("float: string is not a number"))
        }
        other => Err(Error::type_error(format!(
            "float: cannot convert {}",
            other.value_type().name()
        ))),
    }
}

fn builtin_to_bool(args: &[Value]) -> Result<Value> {
    Ok(Value::Bool(arg(args, 0).is_truthy()))
}

fn builtin_abs(args: &[Value]) -> Result<Value> {
    match arg(args, 0) {
        Value::Int(i) => Ok(Value::Int(i.wrapping_abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        other => Err(Error::type_error(format!(
            "abs: expected a number, got {}",
            other.value_type().name()
        ))),
    }
}

fn builtin_min(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    fold_extreme(heap, args, "min", true)
}

fn builtin_max(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    fold_extreme(heap, args, "max", false)
}

fn fold_extreme(_heap: &mut Heap, args: &[Value], name: &str, want_min: bool) -> Result<Value> {
    need_args(name, args, 1)?;
    let mut best = as_number(arg(args, 0), name)?;
    let mut best_val = arg(args, 0);
    for a in &args[1..] {
        let n = as_number(*a, name)?;
        let take = if want_min { n < best } else { n > best };
        if take {
            best = n;
            best_val = *a;
        }
    }
    Ok(best_val)
}

fn builtin_floor(args: &[Value]) -> Result<Value> {
    Ok(Value::Int(as_number(arg(args, 0), "floor")?.floor() as i64))
}

fn builtin_ceil(args: &[Value]) -> Result<Value> {
    Ok(Value::Int(as_number(arg(args, 0), "ceil")?.ceil() as i64))
}

fn builtin_sqrt(args: &[Value]) -> Result<Value> {
    let n = as_number(arg(args, 0), "sqrt")?;
    if n < 0.0 {
        return Err(Error::runtime("sqrt: negative argument"));
    }
    Ok(Value::Float(n.sqrt()))
}

fn builtin_ord(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("ord: expected a string"))?;
    let s = heap.as_str(h)?;
    match s.chars().next() {
        Some(c) => Ok(Value::Int(c as i64)),
        None => Err(Error::runtime("ord: empty string")),
    }
}

fn builtin_chr(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let code = as_int(arg(args, 0), "chr")?;
    let c = u32::try_from(code)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| Error::runtime("chr: invalid code point"))?;
    Ok(Value::Obj(heap.new_string(c.to_string())?))
}

fn builtin_substr(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("substr", args, 2)?;
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("substr: expected a string"))?;
    let start = as_int(arg(args, 1), "substr")?;
    let s = heap.as_str(h)?.to_owned();
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let start = start.clamp(0, n) as usize;
    let end = match args.get(2) {
        Some(v) => as_int(*v, "substr")?.clamp(0, n) as usize,
        None => chars.len(),
    };
    let slice: String = if end > start {
        chars[start..end].iter().collect()
    } else {
        String::new()
    };
    Ok(Value::Obj(heap.new_string(slice)?))
}

fn builtin_case(heap: &mut Heap, args: &[Value], upper: bool) -> Result<Value> {
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("case conversion: expected a string"))?;
    let s = heap.as_str(h)?;
    let converted = if upper { s.to_uppercase() } else { s.to_lowercase() };
    Ok(Value::Obj(heap.new_string(converted)?))
}

fn builtin_push(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("push", args, 2)?;
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("push: expected a list"))?;
    let val = arg(args, 1);
    match heap.get_mut(h)? {
        Object::List(items) => {
            items.push(val);
            Ok(Value::Int(items.len() as i64))
        }
        other => Err(Error::type_error(format!(
            "push: expected a list, got {}",
            other.kind_name()
        ))),
    }
}

fn builtin_pop(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("pop: expected a list"))?;
    match heap.get_mut(h)? {
        Object::List(items) => Ok(items.pop().unwrap_or(Value::Nil)),
        other => Err(Error::type_error(format!(
            "pop: expected a list, got {}",
            other.kind_name()
        ))),
    }
}

fn builtin_keys(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("keys: expected a map"))?;
    let keys: Vec<Value> = match heap.get(h)? {
        Object::Map(pairs) => pairs.iter().map(|(k, _)| *k).collect(),
        other => {
            return Err(Error::type_error(format!(
                "keys: expected a map, got {}",
                other.kind_name()
            )))
        }
    };
    Ok(Value::Obj(heap.new_list(keys)?))
}

fn builtin_values(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("values: expected a map"))?;
    let vals: Vec<Value> = match heap.get(h)? {
        Object::Map(pairs) => pairs.iter().map(|(_, v)| *v).collect(),
        other => {
            return Err(Error::type_error(format!(
                "values: expected a map, got {}",
                other.kind_name()
            )))
        }
    };
    Ok(Value::Obj(heap.new_list(vals)?))
}

fn builtin_contains(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("contains", args, 2)?;
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("contains: expected a list or map"))?;
    let needle = arg(args, 1);
    let found = match heap.get(h)? {
        Object::List(items) => items.iter().any(|v| heap.values_equal(*v, needle)),
        Object::Map(pairs) => pairs.iter().any(|(k, _)| heap.values_equal(*k, needle)),
        Object::Str(s) => {
            if let Value::Obj(nh) = needle {
                if let Object::Str(sub) = heap.get(nh)? {
                    s.contains(sub.as_str())
                } else {
                    false
                }
            } else {
                false
            }
        }
        other => {
            return Err(Error::type_error(format!(
                "contains: unsupported container {}",
                other.kind_name()
            )))
        }
    };
    Ok(Value::Bool(found))
}

fn builtin_range(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("range", args, 1)?;
    let (start, end, step) = match args.len() {
        1 => (0, as_int(arg(args, 0), "range")?, 1),
        2 => (as_int(arg(args, 0), "range")?, as_int(arg(args, 1), "range")?, 1),
        _ => (
            as_int(arg(args, 0), "range")?,
            as_int(arg(args, 1), "range")?,
            as_int(arg(args, 2), "range")?,
        ),
    };
    if step == 0 {
        return Err(Error::runtime("range: step must not be zero"));
    }
    let mut items = Vec::new();
    let mut cur = start;
    while (step > 0 && cur < end) || (step < 0 && cur > end) {
        items.push(Value::Int(cur));
        if items.len() > crate::limits::MAX_COLLECTION_LEN {
            return Err(Error::runtime("range: too many elements"));
        }
        cur = cur.wrapping_add(step);
    }
    Ok(Value::Obj(heap.new_list(items)?))
}

fn builtin_assert(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    if !arg(args, 0).is_truthy() {
        let msg = match args.get(1) {
            Some(v) => heap.display(*v, 4),
            None => "assertion failed".to_string(),
        };
        return Err(Error::runtime(format!("assert: {msg}")));
    }
    Ok(arg(args, 0))
}

fn builtin_repeat(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("repeat", args, 2)?;
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("repeat: expected a string"))?;
    let n = as_int(arg(args, 1), "repeat")?;
    if n < 0 {
        return Err(Error::runtime("repeat: negative count"));
    }
    let s = heap.as_str(h)?.to_owned();
    let total = s.len().saturating_mul(n as usize);
    if total > MAX_STRING_LEN {
        return Err(Error::runtime("repeat: result too large"));
    }
    Ok(Value::Obj(heap.new_string(s.repeat(n as usize))?))
}

fn builtin_join(heap: &mut Heap, args: &[Value]) -> Result<Value> {
    need_args("join", args, 1)?;
    let h = arg(args, 0)
        .as_handle()
        .ok_or_else(|| Error::type_error("join: expected a list"))?;
    let sep = match args.get(1).and_then(|v| v.as_handle()) {
        Some(sh) => heap.as_str(sh)?.to_owned(),
        None => String::new(),
    };
    let items: Vec<Value> = match heap.get(h)? {
        Object::List(items) => items.clone(),
        other => {
            return Err(Error::type_error(format!(
                "join: expected a list, got {}",
                other.kind_name()
            )))
        }
    };
    let mut parts = Vec::with_capacity(items.len());
    for it in items {
        parts.push(heap.display(it, 4));
    }
    Ok(Value::Obj(heap.new_string(parts.join(&sep))?))
}

fn builtin_clamp(args: &[Value]) -> Result<Value> {
    need_args("clamp", args, 3)?;
    let x = as_number(arg(args, 0), "clamp")?;
    let lo = as_number(arg(args, 1), "clamp")?;
    let hi = as_number(arg(args, 2), "clamp")?;
    if lo > hi {
        return Err(Error::runtime("clamp: lower bound above upper bound"));
    }
    Ok(Value::Float(x.clamp(lo, hi)))
}

fn builtin_sign(args: &[Value]) -> Result<Value> {
    let n = as_number(arg(args, 0), "sign")?;
    let s = if n > 0.0 {
        1
    } else if n < 0.0 {
        -1
    } else {
        0
    };
    Ok(Value::Int(s))
}

/// Type predicate used by the compiler's constant folder to reject nonsensical
/// coercions early; exposed for tests.
pub fn coercible_to_number(t: ValueType) -> bool {
    matches!(t, ValueType::Int | ValueType::Float | ValueType::Bool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_is_dense() {
        for (i, (_, id)) in BUILTINS.iter().enumerate() {
            assert_eq!(i as u32, *id, "builtin ids must match their position");
        }
        assert_eq!(BUILTINS.len() as u32, BUILTIN_COUNT);
    }

    #[test]
    fn range_builds_expected_list() {
        let mut h = Heap::new();
        let mut out = String::new();
        let _ = out;
        let v = builtin_range(&mut h, &[Value::Int(3)]).unwrap();
        let handle = v.as_handle().unwrap();
        assert_eq!(h.length_of(handle).unwrap(), 3);
    }

    #[test]
    fn abs_and_sign() {
        assert_eq!(
            builtin_abs(&[Value::Int(-5)]).unwrap().as_int(),
            Some(5)
        );
        assert_eq!(builtin_sign(&[Value::Float(-2.0)]).unwrap().as_int(), Some(-1));
    }
}
