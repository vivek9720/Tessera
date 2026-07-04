//! Resource limits enforced by the loader and virtual machine.
//!
//! Untrusted modules are run inside fixed budgets so that a hostile or merely
//! buggy program fails with a clean [`crate::error::Error`] instead of
//! exhausting memory or looping forever. The numbers are generous for real
//! programs but small enough to keep embedding predictable.

/// Maximum number of registers a single frame may declare. Frames are heap
/// allocated as `Box<[Value]>`, so this bounds per-call memory.
pub const MAX_REGISTERS: u16 = 4096;

/// Maximum number of function prototypes in a module.
pub const MAX_PROTOS: usize = 65_536;

/// Maximum number of constants in a module's pool.
pub const MAX_CONSTS: usize = 262_144;

/// Maximum instructions in a single prototype.
pub const MAX_CODE_LEN: usize = 262_144;

/// Maximum number of upvalues a single closure may declare.
pub const MAX_UPVALUES: usize = 256;

/// Maximum call depth before the VM reports a stack overflow.
pub const MAX_CALL_DEPTH: usize = 256;

/// Maximum number of live heap objects. Reaching this is a runtime error.
pub const MAX_HEAP_OBJECTS: usize = 1 << 20;

/// Maximum number of instructions the VM will execute for one program before
/// giving up (a coarse timeout that does not depend on the wall clock, so runs
/// stay deterministic).
pub const MAX_STEPS: u64 = 50_000_000;

/// Maximum number of elements a single list or map literal may materialize.
pub const MAX_COLLECTION_LEN: usize = 1 << 20;

/// Maximum length in bytes of a string produced by concatenation.
pub const MAX_STRING_LEN: usize = 1 << 24;
