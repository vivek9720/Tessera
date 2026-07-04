//! The object heap.
//!
//! Everything with identity or a variable-size payload lives here and is named
//! by a [`Handle`]. The heap only ever grows during a single program run (it is
//! reclaimed wholesale when the [`crate::vm::Vm`] is dropped), which keeps
//! object lifetimes trivial: a handle is valid for as long as the VM that
//! produced it. Allocation is capped by [`crate::limits::MAX_HEAP_OBJECTS`].
//!
//! Upvalue cells are the one exception to "values are simple". An *open* cell
//! borrows a slot inside a live call frame's register buffer through a raw
//! pointer so that the enclosing function and its closures observe each other's
//! writes. When the frame returns, the cell is *closed*: the current value is
//! copied into the cell and the borrow is dropped. See
//! [`crate::vm::Vm::close_frame_upvalues`] for the lifecycle management.

use crate::error::{Error, Result};
use crate::limits::MAX_HEAP_OBJECTS;
use crate::value::{Handle, Value};

/// An upvalue cell. Open cells point into a frame's registers; closed cells own
/// their value.
#[derive(Debug)]
pub struct UpvalueCell {
    /// Valid only while `closed` is `None`.
    location: *mut Value,
    /// `Some` once the cell has been closed over.
    closed: Option<Value>,
}

impl UpvalueCell {
    fn open(location: *mut Value) -> UpvalueCell {
        UpvalueCell { location, closed: None }
    }

    pub fn is_open(&self) -> bool {
        self.closed.is_none()
    }

    /// The borrowed slot address, used by the frame-teardown logic to decide
    /// whether this cell belongs to the frame being unwound.
    pub fn location_addr(&self) -> usize {
        self.location as usize
    }

    /// Read the current value, following the borrow when still open.
    ///
    /// # Safety of the open path
    ///
    /// While `closed` is `None`, `location` is required to point at a live
    /// register slot. The VM upholds that by closing every cell that borrows a
    /// frame before that frame's register buffer is released.
    pub fn get(&self) -> Value {
        match self.closed {
            Some(v) => v,
            None => unsafe { *self.location },
        }
    }

    /// Write through the cell.
    pub fn set(&mut self, v: Value) {
        match &mut self.closed {
            Some(slot) => *slot = v,
            None => unsafe {
                *self.location = v;
            },
        }
    }

    /// Copy the borrowed value into the cell and drop the borrow.
    pub fn close(&mut self) {
        if self.closed.is_none() {
            let v = unsafe { *self.location };
            self.closed = Some(v);
            self.location = std::ptr::null_mut();
        }
    }
}

/// A closure: a prototype paired with captured upvalue cells.
#[derive(Debug, Clone)]
pub struct Closure {
    pub proto: u32,
    pub upvalues: Vec<Handle>,
}

/// A heap-resident object.
#[derive(Debug)]
pub enum Object {
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Closure(Closure),
    Upvalue(UpvalueCell),
    /// A host-provided native function, named by an index into the builtin
    /// table registered on the VM.
    Native(u32),
}

impl Object {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Object::Str(_) => "str",
            Object::Bytes(_) => "bytes",
            Object::List(_) => "list",
            Object::Map(_) => "map",
            Object::Closure(_) => "function",
            Object::Upvalue(_) => "upvalue",
            Object::Native(_) => "function",
        }
    }
}

/// The object store.
pub struct Heap {
    objects: Vec<Object>,
}

impl Heap {
    pub fn new() -> Heap {
        Heap { objects: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    fn alloc(&mut self, obj: Object) -> Result<Handle> {
        if self.objects.len() >= MAX_HEAP_OBJECTS {
            return Err(Error::runtime("heap object limit exceeded"));
        }
        let handle = Handle(self.objects.len() as u32);
        self.objects.push(obj);
        Ok(handle)
    }

    // ---- Typed constructors ------------------------------------------------

    pub fn new_string(&mut self, s: impl Into<String>) -> Result<Handle> {
        self.alloc(Object::Str(s.into()))
    }

    pub fn new_bytes(&mut self, b: Vec<u8>) -> Result<Handle> {
        self.alloc(Object::Bytes(b))
    }

    pub fn new_list(&mut self, items: Vec<Value>) -> Result<Handle> {
        self.alloc(Object::List(items))
    }

    pub fn new_map(&mut self, pairs: Vec<(Value, Value)>) -> Result<Handle> {
        self.alloc(Object::Map(pairs))
    }

    pub fn new_closure(&mut self, proto: u32, upvalues: Vec<Handle>) -> Result<Handle> {
        self.alloc(Object::Closure(Closure { proto, upvalues }))
    }

    pub fn new_open_upvalue(&mut self, location: *mut Value) -> Result<Handle> {
        self.alloc(Object::Upvalue(UpvalueCell::open(location)))
    }

    pub fn new_native(&mut self, id: u32) -> Result<Handle> {
        self.alloc(Object::Native(id))
    }

    // ---- Accessors ---------------------------------------------------------

    pub fn get(&self, h: Handle) -> Result<&Object> {
        self.objects
            .get(h.index())
            .ok_or_else(|| Error::runtime("dangling object handle"))
    }

    pub fn get_mut(&mut self, h: Handle) -> Result<&mut Object> {
        self.objects
            .get_mut(h.index())
            .ok_or_else(|| Error::runtime("dangling object handle"))
    }

    pub fn as_str(&self, h: Handle) -> Result<&str> {
        match self.get(h)? {
            Object::Str(s) => Ok(s),
            other => Err(Error::type_error(format!(
                "expected str, found {}",
                other.kind_name()
            ))),
        }
    }

    pub fn as_closure(&self, h: Handle) -> Result<&Closure> {
        match self.get(h)? {
            Object::Closure(c) => Ok(c),
            other => Err(Error::type_error(format!(
                "expected function, found {}",
                other.kind_name()
            ))),
        }
    }

    // ---- Upvalue operations ------------------------------------------------

    /// Read through an upvalue cell.
    pub fn upvalue_get(&self, h: Handle) -> Result<Value> {
        match self.get(h)? {
            Object::Upvalue(cell) => Ok(cell.get()),
            other => Err(Error::runtime(format!(
                "expected upvalue, found {}",
                other.kind_name()
            ))),
        }
    }

    /// Write through an upvalue cell.
    pub fn upvalue_set(&mut self, h: Handle, v: Value) -> Result<()> {
        match self.get_mut(h)? {
            Object::Upvalue(cell) => {
                cell.set(v);
                Ok(())
            }
            other => Err(Error::runtime(format!(
                "expected upvalue, found {}",
                other.kind_name()
            ))),
        }
    }

    /// Address of the register slot an open cell borrows, or `None` if closed or
    /// not an upvalue.
    pub fn upvalue_addr(&self, h: Handle) -> Option<usize> {
        match self.objects.get(h.index()) {
            Some(Object::Upvalue(cell)) if cell.is_open() => Some(cell.location_addr()),
            _ => None,
        }
    }

    /// Close an open upvalue cell, copying its borrowed value inline.
    pub fn upvalue_close(&mut self, h: Handle) {
        if let Some(Object::Upvalue(cell)) = self.objects.get_mut(h.index()) {
            cell.close();
        }
    }

    // ---- Length / structural helpers --------------------------------------

    /// Length of a string (bytes), byte string, list, or map.
    pub fn length_of(&self, h: Handle) -> Result<i64> {
        let n = match self.get(h)? {
            Object::Str(s) => s.len(),
            Object::Bytes(b) => b.len(),
            Object::List(l) => l.len(),
            Object::Map(m) => m.len(),
            other => {
                return Err(Error::type_error(format!(
                    "value of type {} has no length",
                    other.kind_name()
                )))
            }
        };
        Ok(n as i64)
    }

    /// Structural-ish equality: scalars compare by value, strings/bytes by
    /// content, and aggregates by identity to avoid unbounded recursion over
    /// possibly-cyclic structures.
    pub fn values_equal(&self, a: Value, b: Value) -> bool {
        match (a, b) {
            (Value::Obj(ha), Value::Obj(hb)) => {
                if ha == hb {
                    return true;
                }
                match (self.objects.get(ha.index()), self.objects.get(hb.index())) {
                    (Some(Object::Str(x)), Some(Object::Str(y))) => x == y,
                    (Some(Object::Bytes(x)), Some(Object::Bytes(y))) => x == y,
                    _ => false,
                }
            }
            _ => a.scalar_eq(&b),
        }
    }

    /// Render a value to text, following aggregates up to `depth` levels.
    pub fn display(&self, v: Value, depth: usize) -> String {
        match v {
            Value::Obj(h) => match self.objects.get(h.index()) {
                Some(Object::Str(s)) => s.clone(),
                Some(Object::Bytes(b)) => format!("<bytes len={}>", b.len()),
                Some(Object::List(items)) => {
                    if depth == 0 {
                        return "[...]".to_string();
                    }
                    let parts: Vec<String> =
                        items.iter().map(|it| self.display(*it, depth - 1)).collect();
                    format!("[{}]", parts.join(", "))
                }
                Some(Object::Map(pairs)) => {
                    if depth == 0 {
                        return "{...}".to_string();
                    }
                    let parts: Vec<String> = pairs
                        .iter()
                        .map(|(k, val)| {
                            format!(
                                "{}: {}",
                                self.display(*k, depth - 1),
                                self.display(*val, depth - 1)
                            )
                        })
                        .collect();
                    format!("{{{}}}", parts.join(", "))
                }
                Some(Object::Closure(c)) => format!("<function #{}>", c.proto),
                Some(Object::Upvalue(_)) => "<upvalue>".to_string(),
                Some(Object::Native(id)) => format!("<native #{id}>"),
                None => "<dangling>".to_string(),
            },
            other => other.to_string(),
        }
    }
}

impl Default for Heap {
    fn default() -> Heap {
        Heap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_lists_report_length() {
        let mut h = Heap::new();
        let s = h.new_string("hello").unwrap();
        assert_eq!(h.length_of(s).unwrap(), 5);
        let l = h.new_list(vec![Value::Int(1), Value::Int(2)]).unwrap();
        assert_eq!(h.length_of(l).unwrap(), 2);
    }

    #[test]
    fn open_upvalue_follows_and_closes() {
        let mut h = Heap::new();
        let mut slot = Value::Int(10);
        let cell = h.new_open_upvalue(&mut slot as *mut Value).unwrap();
        assert_eq!(h.upvalue_get(cell).unwrap().as_int(), Some(10));
        // Writing through the cell updates the borrowed slot.
        h.upvalue_set(cell, Value::Int(20)).unwrap();
        assert_eq!(slot.as_int(), Some(20));
        // Closing snapshots the value; later changes to the slot are ignored.
        h.upvalue_close(cell);
        slot = Value::Int(30);
        let _ = slot;
        assert_eq!(h.upvalue_get(cell).unwrap().as_int(), Some(20));
    }

    #[test]
    fn string_equality_by_content() {
        let mut h = Heap::new();
        let a = h.new_string("abc").unwrap();
        let b = h.new_string("abc").unwrap();
        assert!(h.values_equal(Value::Obj(a), Value::Obj(b)));
    }
}
