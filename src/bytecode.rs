//! The Tessera instruction set and its on-disk encoding.
//!
//! Tessera compiles to a register machine. Each function prototype declares a
//! register count; instructions address registers by index and never touch the
//! machine's operand stack directly. Control flow uses *instruction indices*
//! rather than byte offsets, which keeps the jump-resolution logic in the VM
//! trivially in-bounds once the verifier has range-checked every target.
//!
//! ## Frame register layout
//!
//! Register 0 of every frame is reserved for the executing closure itself (the
//! "self" slot); user parameters and locals are allocated starting at register
//! 1. Keeping slot 0 pinned lets recursive calls and error reporting recover the
//! active function without walking the call stack. The compiler therefore never
//! emits an upvalue capture of register 0 — user code cannot name the self slot.
//!
//! Instructions are decoded up front into the [`Instr`] enum; the interpreter
//! executes the decoded form, so the byte layout below matters only to the
//! loader and serializer.

use crate::error::{Error, Result};
use crate::reader::Reader;
use crate::writer::Writer;

/// A register index within a frame. Frames are capped well below `u16::MAX`.
pub type Reg = u16;
/// An index into a function's constant pool.
pub type ConstIdx = u32;
/// An index into a module's function-prototype table.
pub type ProtoIdx = u32;
/// An index into a closure's upvalue list.
pub type UpvalIdx = u16;
/// An instruction index within a function's code array (a jump target).
pub type CodeAddr = u32;

/// One-byte opcode tags as they appear in a serialized module.
pub mod op {
    pub const LOAD_NIL: u8 = 0x00;
    pub const LOAD_TRUE: u8 = 0x01;
    pub const LOAD_FALSE: u8 = 0x02;
    pub const LOAD_CONST: u8 = 0x03;
    pub const LOAD_INT: u8 = 0x04;
    pub const MOVE: u8 = 0x05;

    pub const ADD: u8 = 0x10;
    pub const SUB: u8 = 0x11;
    pub const MUL: u8 = 0x12;
    pub const DIV: u8 = 0x13;
    pub const MOD: u8 = 0x14;
    pub const NEG: u8 = 0x15;
    pub const POW: u8 = 0x16;

    pub const EQ: u8 = 0x20;
    pub const NE: u8 = 0x21;
    pub const LT: u8 = 0x22;
    pub const LE: u8 = 0x23;
    pub const GT: u8 = 0x24;
    pub const GE: u8 = 0x25;
    pub const NOT: u8 = 0x26;

    pub const JUMP: u8 = 0x30;
    pub const JUMP_IF_FALSE: u8 = 0x31;
    pub const JUMP_IF_TRUE: u8 = 0x32;

    pub const CALL: u8 = 0x40;
    pub const RETURN: u8 = 0x41;
    pub const TAIL_CALL: u8 = 0x42;

    pub const MAKE_LIST: u8 = 0x50;
    pub const MAKE_MAP: u8 = 0x51;
    pub const INDEX: u8 = 0x52;
    pub const SET_INDEX: u8 = 0x53;
    pub const LEN: u8 = 0x54;
    pub const APPEND: u8 = 0x55;
    pub const CONCAT: u8 = 0x56;

    pub const GET_GLOBAL: u8 = 0x60;
    pub const SET_GLOBAL: u8 = 0x61;
    pub const GET_UPVAL: u8 = 0x62;
    pub const SET_UPVAL: u8 = 0x63;
    pub const CLOSURE: u8 = 0x64;

    pub const TYPE_OF: u8 = 0x70;
    pub const NOP: u8 = 0x71;
    pub const HALT: u8 = 0x72;
}

/// A decoded instruction with typed operands.
///
/// Register operands are named by their role (`dst`, `a`, `b`, ...). Window
/// operands (`base` + `count`) address a contiguous run of registers used to
/// gather call arguments and collection elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instr {
    /// `dst = nil`
    LoadNil { dst: Reg },
    /// `dst = true`
    LoadTrue { dst: Reg },
    /// `dst = false`
    LoadFalse { dst: Reg },
    /// `dst = consts[k]`
    LoadConst { dst: Reg, k: ConstIdx },
    /// `dst = imm` (small signed immediate, avoids a constant-pool entry)
    LoadInt { dst: Reg, imm: i32 },
    /// `dst = src`
    Move { dst: Reg, src: Reg },

    Add { dst: Reg, a: Reg, b: Reg },
    Sub { dst: Reg, a: Reg, b: Reg },
    Mul { dst: Reg, a: Reg, b: Reg },
    Div { dst: Reg, a: Reg, b: Reg },
    Mod { dst: Reg, a: Reg, b: Reg },
    Neg { dst: Reg, a: Reg },
    Pow { dst: Reg, a: Reg, b: Reg },

    Eq { dst: Reg, a: Reg, b: Reg },
    Ne { dst: Reg, a: Reg, b: Reg },
    Lt { dst: Reg, a: Reg, b: Reg },
    Le { dst: Reg, a: Reg, b: Reg },
    Gt { dst: Reg, a: Reg, b: Reg },
    Ge { dst: Reg, a: Reg, b: Reg },
    Not { dst: Reg, a: Reg },

    /// Unconditional jump to instruction `target`.
    Jump { target: CodeAddr },
    /// Jump to `target` when `cond` is falsey.
    JumpIfFalse { cond: Reg, target: CodeAddr },
    /// Jump to `target` when `cond` is truthy.
    JumpIfTrue { cond: Reg, target: CodeAddr },

    /// `dst = callee(base .. base+argc)`
    Call { dst: Reg, callee: Reg, base: Reg, argc: u16 },
    /// Tail call: replace the current frame with `callee(base .. base+argc)`.
    TailCall { callee: Reg, base: Reg, argc: u16 },
    /// Return register `src` to the caller.
    Return { src: Reg },

    /// `dst = [base .. base+count]`
    MakeList { dst: Reg, base: Reg, count: u16 },
    /// `dst = { base[0]:base[1], ... }`, reading `count` key/value pairs.
    MakeMap { dst: Reg, base: Reg, count: u16 },
    /// `dst = obj[key]`
    Index { dst: Reg, obj: Reg, key: Reg },
    /// `obj[key] = val`
    SetIndex { obj: Reg, key: Reg, val: Reg },
    /// `dst = len(obj)`
    Len { dst: Reg, obj: Reg },
    /// `obj.append(val)` for lists.
    Append { obj: Reg, val: Reg },
    /// `dst = a ++ b` (string/list concatenation).
    Concat { dst: Reg, a: Reg, b: Reg },

    /// `dst = globals[consts[name]]`
    GetGlobal { dst: Reg, name: ConstIdx },
    /// `globals[consts[name]] = src`
    SetGlobal { name: ConstIdx, src: Reg },
    /// `dst = upvalues[uv]`
    GetUpval { dst: Reg, uv: UpvalIdx },
    /// `upvalues[uv] = src`
    SetUpval { uv: UpvalIdx, src: Reg },
    /// `dst = closure(protos[proto])`, capturing upvalues per the prototype.
    Closure { dst: Reg, proto: ProtoIdx },

    /// `dst = type_name(a)`
    TypeOf { dst: Reg, a: Reg },
    /// No operation.
    Nop,
    /// Stop the VM, yielding register 0 of the top frame as the program result.
    Halt,
}

impl Instr {
    /// The one-byte tag this instruction serializes to.
    pub fn opcode(&self) -> u8 {
        match self {
            Instr::LoadNil { .. } => op::LOAD_NIL,
            Instr::LoadTrue { .. } => op::LOAD_TRUE,
            Instr::LoadFalse { .. } => op::LOAD_FALSE,
            Instr::LoadConst { .. } => op::LOAD_CONST,
            Instr::LoadInt { .. } => op::LOAD_INT,
            Instr::Move { .. } => op::MOVE,
            Instr::Add { .. } => op::ADD,
            Instr::Sub { .. } => op::SUB,
            Instr::Mul { .. } => op::MUL,
            Instr::Div { .. } => op::DIV,
            Instr::Mod { .. } => op::MOD,
            Instr::Neg { .. } => op::NEG,
            Instr::Pow { .. } => op::POW,
            Instr::Eq { .. } => op::EQ,
            Instr::Ne { .. } => op::NE,
            Instr::Lt { .. } => op::LT,
            Instr::Le { .. } => op::LE,
            Instr::Gt { .. } => op::GT,
            Instr::Ge { .. } => op::GE,
            Instr::Not { .. } => op::NOT,
            Instr::Jump { .. } => op::JUMP,
            Instr::JumpIfFalse { .. } => op::JUMP_IF_FALSE,
            Instr::JumpIfTrue { .. } => op::JUMP_IF_TRUE,
            Instr::Call { .. } => op::CALL,
            Instr::TailCall { .. } => op::TAIL_CALL,
            Instr::Return { .. } => op::RETURN,
            Instr::MakeList { .. } => op::MAKE_LIST,
            Instr::MakeMap { .. } => op::MAKE_MAP,
            Instr::Index { .. } => op::INDEX,
            Instr::SetIndex { .. } => op::SET_INDEX,
            Instr::Len { .. } => op::LEN,
            Instr::Append { .. } => op::APPEND,
            Instr::Concat { .. } => op::CONCAT,
            Instr::GetGlobal { .. } => op::GET_GLOBAL,
            Instr::SetGlobal { .. } => op::SET_GLOBAL,
            Instr::GetUpval { .. } => op::GET_UPVAL,
            Instr::SetUpval { .. } => op::SET_UPVAL,
            Instr::Closure { .. } => op::CLOSURE,
            Instr::TypeOf { .. } => op::TYPE_OF,
            Instr::Nop => op::NOP,
            Instr::Halt => op::HALT,
        }
    }

    /// A short mnemonic used by the disassembler.
    pub fn mnemonic(&self) -> &'static str {
        match self {
            Instr::LoadNil { .. } => "loadnil",
            Instr::LoadTrue { .. } => "loadtrue",
            Instr::LoadFalse { .. } => "loadfalse",
            Instr::LoadConst { .. } => "loadk",
            Instr::LoadInt { .. } => "loadi",
            Instr::Move { .. } => "move",
            Instr::Add { .. } => "add",
            Instr::Sub { .. } => "sub",
            Instr::Mul { .. } => "mul",
            Instr::Div { .. } => "div",
            Instr::Mod { .. } => "mod",
            Instr::Neg { .. } => "neg",
            Instr::Pow { .. } => "pow",
            Instr::Eq { .. } => "eq",
            Instr::Ne { .. } => "ne",
            Instr::Lt { .. } => "lt",
            Instr::Le { .. } => "le",
            Instr::Gt { .. } => "gt",
            Instr::Ge { .. } => "ge",
            Instr::Not { .. } => "not",
            Instr::Jump { .. } => "jmp",
            Instr::JumpIfFalse { .. } => "jmpf",
            Instr::JumpIfTrue { .. } => "jmpt",
            Instr::Call { .. } => "call",
            Instr::TailCall { .. } => "tailcall",
            Instr::Return { .. } => "ret",
            Instr::MakeList { .. } => "newlist",
            Instr::MakeMap { .. } => "newmap",
            Instr::Index { .. } => "index",
            Instr::SetIndex { .. } => "setindex",
            Instr::Len { .. } => "len",
            Instr::Append { .. } => "append",
            Instr::Concat { .. } => "concat",
            Instr::GetGlobal { .. } => "getglobal",
            Instr::SetGlobal { .. } => "setglobal",
            Instr::GetUpval { .. } => "getupval",
            Instr::SetUpval { .. } => "setupval",
            Instr::Closure { .. } => "closure",
            Instr::TypeOf { .. } => "typeof",
            Instr::Nop => "nop",
            Instr::Halt => "halt",
        }
    }

    /// Serialize this instruction (opcode byte + operands) into `w`.
    pub fn encode(&self, w: &mut Writer) {
        w.u8(self.opcode());
        match *self {
            Instr::LoadNil { dst }
            | Instr::LoadTrue { dst }
            | Instr::LoadFalse { dst } => {
                w.uleb(dst as u64);
            }
            Instr::LoadConst { dst, k } => {
                w.uleb(dst as u64).uleb(k as u64);
            }
            Instr::LoadInt { dst, imm } => {
                w.uleb(dst as u64).uleb(zigzag(imm as i64));
            }
            Instr::Move { dst, src } => {
                w.uleb(dst as u64).uleb(src as u64);
            }
            Instr::Add { dst, a, b }
            | Instr::Sub { dst, a, b }
            | Instr::Mul { dst, a, b }
            | Instr::Div { dst, a, b }
            | Instr::Mod { dst, a, b }
            | Instr::Pow { dst, a, b }
            | Instr::Eq { dst, a, b }
            | Instr::Ne { dst, a, b }
            | Instr::Lt { dst, a, b }
            | Instr::Le { dst, a, b }
            | Instr::Gt { dst, a, b }
            | Instr::Ge { dst, a, b }
            | Instr::Concat { dst, a, b } => {
                w.uleb(dst as u64).uleb(a as u64).uleb(b as u64);
            }
            Instr::Neg { dst, a } | Instr::Not { dst, a } | Instr::TypeOf { dst, a } => {
                w.uleb(dst as u64).uleb(a as u64);
            }
            Instr::Jump { target } => {
                w.uleb(target as u64);
            }
            Instr::JumpIfFalse { cond, target } | Instr::JumpIfTrue { cond, target } => {
                w.uleb(cond as u64).uleb(target as u64);
            }
            Instr::Call { dst, callee, base, argc } => {
                w.uleb(dst as u64)
                    .uleb(callee as u64)
                    .uleb(base as u64)
                    .uleb(argc as u64);
            }
            Instr::TailCall { callee, base, argc } => {
                w.uleb(callee as u64).uleb(base as u64).uleb(argc as u64);
            }
            Instr::Return { src } => {
                w.uleb(src as u64);
            }
            Instr::MakeList { dst, base, count } | Instr::MakeMap { dst, base, count } => {
                w.uleb(dst as u64).uleb(base as u64).uleb(count as u64);
            }
            Instr::Index { dst, obj, key } => {
                w.uleb(dst as u64).uleb(obj as u64).uleb(key as u64);
            }
            Instr::SetIndex { obj, key, val } => {
                w.uleb(obj as u64).uleb(key as u64).uleb(val as u64);
            }
            Instr::Len { dst, obj } => {
                w.uleb(dst as u64).uleb(obj as u64);
            }
            Instr::Append { obj, val } => {
                w.uleb(obj as u64).uleb(val as u64);
            }
            Instr::GetGlobal { dst, name } => {
                w.uleb(dst as u64).uleb(name as u64);
            }
            Instr::SetGlobal { name, src } => {
                w.uleb(name as u64).uleb(src as u64);
            }
            Instr::GetUpval { dst, uv } => {
                w.uleb(dst as u64).uleb(uv as u64);
            }
            Instr::SetUpval { uv, src } => {
                w.uleb(uv as u64).uleb(src as u64);
            }
            Instr::Closure { dst, proto } => {
                w.uleb(dst as u64).uleb(proto as u64);
            }
            Instr::Nop | Instr::Halt => {}
        }
    }

    /// Decode a single instruction from `r`.
    ///
    /// Register-shaped operands are narrowed to their declared widths here;
    /// values that do not fit (for example a register index above `u16::MAX`)
    /// are rejected as malformed rather than silently truncated. Semantic range
    /// checks against the owning function happen later, in the verifier.
    pub fn decode(r: &mut Reader) -> Result<Instr> {
        let opcode = r.u8()?;
        let reg = |r: &mut Reader| -> Result<Reg> {
            let v = r.uleb()?;
            Reg::try_from(v).map_err(|_| Error::decode("register operand out of range"))
        };
        let count = |r: &mut Reader| -> Result<u16> {
            let v = r.uleb()?;
            u16::try_from(v).map_err(|_| Error::decode("window operand out of range"))
        };
        let cidx = |r: &mut Reader| -> Result<ConstIdx> {
            let v = r.uleb()?;
            ConstIdx::try_from(v).map_err(|_| Error::decode("constant index out of range"))
        };
        let addr = |r: &mut Reader| -> Result<CodeAddr> {
            let v = r.uleb()?;
            CodeAddr::try_from(v).map_err(|_| Error::decode("jump target out of range"))
        };

        let instr = match opcode {
            op::LOAD_NIL => Instr::LoadNil { dst: reg(r)? },
            op::LOAD_TRUE => Instr::LoadTrue { dst: reg(r)? },
            op::LOAD_FALSE => Instr::LoadFalse { dst: reg(r)? },
            op::LOAD_CONST => Instr::LoadConst { dst: reg(r)?, k: cidx(r)? },
            op::LOAD_INT => {
                let dst = reg(r)?;
                let imm = unzigzag(r.uleb()?);
                let imm = i32::try_from(imm)
                    .map_err(|_| Error::decode("immediate out of range"))?;
                Instr::LoadInt { dst, imm }
            }
            op::MOVE => Instr::Move { dst: reg(r)?, src: reg(r)? },
            op::ADD => Instr::Add { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::SUB => Instr::Sub { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::MUL => Instr::Mul { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::DIV => Instr::Div { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::MOD => Instr::Mod { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::NEG => Instr::Neg { dst: reg(r)?, a: reg(r)? },
            op::POW => Instr::Pow { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::EQ => Instr::Eq { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::NE => Instr::Ne { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::LT => Instr::Lt { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::LE => Instr::Le { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::GT => Instr::Gt { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::GE => Instr::Ge { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::NOT => Instr::Not { dst: reg(r)?, a: reg(r)? },
            op::JUMP => Instr::Jump { target: addr(r)? },
            op::JUMP_IF_FALSE => Instr::JumpIfFalse { cond: reg(r)?, target: addr(r)? },
            op::JUMP_IF_TRUE => Instr::JumpIfTrue { cond: reg(r)?, target: addr(r)? },
            op::CALL => Instr::Call {
                dst: reg(r)?,
                callee: reg(r)?,
                base: reg(r)?,
                argc: count(r)?,
            },
            op::TAIL_CALL => Instr::TailCall {
                callee: reg(r)?,
                base: reg(r)?,
                argc: count(r)?,
            },
            op::RETURN => Instr::Return { src: reg(r)? },
            op::MAKE_LIST => Instr::MakeList { dst: reg(r)?, base: reg(r)?, count: count(r)? },
            op::MAKE_MAP => Instr::MakeMap { dst: reg(r)?, base: reg(r)?, count: count(r)? },
            op::INDEX => Instr::Index { dst: reg(r)?, obj: reg(r)?, key: reg(r)? },
            op::SET_INDEX => Instr::SetIndex { obj: reg(r)?, key: reg(r)?, val: reg(r)? },
            op::LEN => Instr::Len { dst: reg(r)?, obj: reg(r)? },
            op::APPEND => Instr::Append { obj: reg(r)?, val: reg(r)? },
            op::CONCAT => Instr::Concat { dst: reg(r)?, a: reg(r)?, b: reg(r)? },
            op::GET_GLOBAL => Instr::GetGlobal { dst: reg(r)?, name: cidx(r)? },
            op::SET_GLOBAL => Instr::SetGlobal { name: cidx(r)?, src: reg(r)? },
            op::GET_UPVAL => {
                let dst = reg(r)?;
                let uv = count(r)?;
                Instr::GetUpval { dst, uv }
            }
            op::SET_UPVAL => {
                let uv = count(r)?;
                let src = reg(r)?;
                Instr::SetUpval { uv, src }
            }
            op::CLOSURE => {
                let dst = reg(r)?;
                let proto = r.uleb()?;
                let proto = ProtoIdx::try_from(proto)
                    .map_err(|_| Error::decode("prototype index out of range"))?;
                Instr::Closure { dst, proto }
            }
            op::TYPE_OF => Instr::TypeOf { dst: reg(r)?, a: reg(r)? },
            op::NOP => Instr::Nop,
            op::HALT => Instr::Halt,
            other => {
                return Err(Error::decode(format!("unknown opcode {:#04x}", other)));
            }
        };
        Ok(instr)
    }
}

/// Zig-zag encode a signed integer so small magnitudes stay short under LEB128.
pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Inverse of [`zigzag`].
pub fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(instr: Instr) {
        let mut w = Writer::new();
        instr.encode(&mut w);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let decoded = Instr::decode(&mut r).unwrap();
        assert_eq!(decoded, instr);
        assert!(r.at_end(), "decoder left {} trailing bytes", r.remaining());
    }

    #[test]
    fn zigzag_roundtrips() {
        for v in [0i64, 1, -1, 2, -2, i32::MAX as i64, i32::MIN as i64] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn every_shape_roundtrips() {
        roundtrip(Instr::LoadNil { dst: 3 });
        roundtrip(Instr::LoadConst { dst: 1, k: 40000 });
        roundtrip(Instr::LoadInt { dst: 2, imm: -12345 });
        roundtrip(Instr::Move { dst: 1, src: 2 });
        roundtrip(Instr::Add { dst: 1, a: 2, b: 3 });
        roundtrip(Instr::Neg { dst: 1, a: 2 });
        roundtrip(Instr::Jump { target: 99 });
        roundtrip(Instr::JumpIfFalse { cond: 4, target: 7 });
        roundtrip(Instr::Call { dst: 1, callee: 2, base: 3, argc: 4 });
        roundtrip(Instr::TailCall { callee: 2, base: 3, argc: 4 });
        roundtrip(Instr::MakeList { dst: 1, base: 2, count: 5 });
        roundtrip(Instr::MakeMap { dst: 1, base: 2, count: 3 });
        roundtrip(Instr::Index { dst: 1, obj: 2, key: 3 });
        roundtrip(Instr::SetIndex { obj: 1, key: 2, val: 3 });
        roundtrip(Instr::GetGlobal { dst: 1, name: 12 });
        roundtrip(Instr::SetGlobal { name: 12, src: 1 });
        roundtrip(Instr::GetUpval { dst: 1, uv: 2 });
        roundtrip(Instr::SetUpval { uv: 2, src: 1 });
        roundtrip(Instr::Closure { dst: 1, proto: 6 });
        roundtrip(Instr::TypeOf { dst: 1, a: 2 });
        roundtrip(Instr::Nop);
        roundtrip(Instr::Halt);
    }

    #[test]
    fn unknown_opcode_rejected() {
        let mut r = Reader::new(&[0xff]);
        assert!(Instr::decode(&mut r).is_err());
    }
}
