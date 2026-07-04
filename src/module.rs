//! In-memory representation of a compiled Tessera module.
//!
//! A module bundles a shared constant pool with a table of function prototypes.
//! Each [`Proto`] carries its own code array, register count, arity, and the
//! descriptors used to wire up its closures' upvalues when it is instantiated.
//! The [`crate::loader`] produces a `Module` from bytes and the
//! [`crate::serialize`] module writes one back out.

use crate::bytecode::{Instr, Reg, UpvalIdx};

/// A constant-pool entry. Booleans and nil have dedicated load opcodes and are
/// not stored here; the pool holds only values with a payload.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i64),
    Float(f64),
    Str(String),
    /// A raw byte string (used for binary blob literals).
    Bytes(Vec<u8>),
}

impl Const {
    pub fn type_name(&self) -> &'static str {
        match self {
            Const::Int(_) => "int",
            Const::Float(_) => "float",
            Const::Str(_) => "str",
            Const::Bytes(_) => "bytes",
        }
    }
}

/// How a closure obtains one of its upvalues when it is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpvalDesc {
    /// Capture register `reg` of the *enclosing* frame. If several closures
    /// capture the same register, they share one open upvalue cell so that
    /// writes are observed by all of them until the enclosing frame returns.
    FromLocal(Reg),
    /// Inherit upvalue `idx` from the enclosing closure.
    FromUpval(UpvalIdx),
}

impl UpvalDesc {
    pub fn is_local(&self) -> bool {
        matches!(self, UpvalDesc::FromLocal(_))
    }
}

/// A function prototype: the immutable template a closure is instantiated from.
#[derive(Debug, Clone, PartialEq)]
pub struct Proto {
    /// Human-readable name, for diagnostics and disassembly.
    pub name: String,
    /// Number of declared parameters. Arguments beyond this are dropped and
    /// missing arguments are filled with `nil`.
    pub arity: u16,
    /// Total registers this frame needs, including the reserved self slot at
    /// register 0, parameters, locals, and temporaries.
    pub reg_count: u16,
    /// Whether the function accepts a trailing variadic argument list.
    pub is_variadic: bool,
    /// The instruction stream.
    pub code: Vec<Instr>,
    /// Upvalue wiring, one descriptor per upvalue the closure will hold.
    pub upvals: Vec<UpvalDesc>,
}

impl Proto {
    pub fn new(name: impl Into<String>) -> Proto {
        Proto {
            name: name.into(),
            arity: 0,
            reg_count: 1, // register 0 is always reserved
            is_variadic: false,
            code: Vec::new(),
            upvals: Vec::new(),
        }
    }

    /// The number of upvalues this prototype's closures carry.
    pub fn upvalue_count(&self) -> usize {
        self.upvals.len()
    }

    /// Number of instructions in the code array.
    pub fn code_len(&self) -> usize {
        self.code.len()
    }
}

/// The unit produced by compilation and consumed by the VM.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Format minor version this module was produced for.
    pub version: u16,
    /// Optional module name (the source file stem, usually).
    pub name: String,
    /// Shared constant pool, indexed by `LoadConst`, `GetGlobal`, etc.
    pub consts: Vec<Const>,
    /// Function prototypes. `protos[entry]` is the module's entry point.
    pub protos: Vec<Proto>,
    /// Index of the top-level function to execute first.
    pub entry: u32,
}

impl Module {
    pub fn new() -> Module {
        Module {
            version: crate::MODULE_VERSION,
            name: String::new(),
            consts: Vec::new(),
            protos: Vec::new(),
            entry: 0,
        }
    }

    /// Append a constant, returning its pool index. Existing equal constants are
    /// reused so the pool stays compact.
    pub fn intern_const(&mut self, c: Const) -> u32 {
        if let Some(pos) = self.consts.iter().position(|existing| existing == &c) {
            return pos as u32;
        }
        let idx = self.consts.len() as u32;
        self.consts.push(c);
        idx
    }

    /// Append a prototype, returning its table index.
    pub fn add_proto(&mut self, proto: Proto) -> u32 {
        let idx = self.protos.len() as u32;
        self.protos.push(proto);
        idx
    }

    pub fn entry_proto(&self) -> Option<&Proto> {
        self.protos.get(self.entry as usize)
    }

    /// Total number of instructions across all prototypes, used for coarse size
    /// reporting and fuzzing budget heuristics.
    pub fn total_instructions(&self) -> usize {
        self.protos.iter().map(|p| p.code.len()).sum()
    }
}

impl Default for Module {
    fn default() -> Module {
        Module::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_reuses_equal_constants() {
        let mut m = Module::new();
        let a = m.intern_const(Const::Int(7));
        let b = m.intern_const(Const::Int(7));
        let c = m.intern_const(Const::Str("x".into()));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(m.consts.len(), 2);
    }

    #[test]
    fn new_proto_reserves_slot_zero() {
        let p = Proto::new("main");
        assert_eq!(p.reg_count, 1);
        assert_eq!(p.arity, 0);
    }
}
