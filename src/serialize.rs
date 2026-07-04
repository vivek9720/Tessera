//! Serialization of a [`Module`] to the Tessera binary module format.
//!
//! The layout, in order:
//!
//! ```text
//! magic      : 4 bytes  "TSRA"
//! version    : u16      format minor version
//! flags      : u16      reserved, currently zero
//! name       : str      module name (length-prefixed UTF-8)
//! consts     : uleb count, then <tag u8><payload> per entry
//! protos     : uleb count, then <proto> per entry
//! entry      : uleb      index of the entry-point prototype
//! ```
//!
//! Each prototype is:
//!
//! ```text
//! name       : str
//! arity      : uleb
//! reg_count  : uleb
//! flags      : u8        bit 0 = variadic
//! upvals     : uleb count, then <tag u8><index uleb> per entry
//! code       : uleb count, then <instruction> per entry
//! ```
//!
//! The reader in [`crate::loader`] mirrors this exactly.

use crate::module::{Const, Module, Proto, UpvalDesc};
use crate::writer::Writer;

/// Constant-pool entry tags.
pub const CONST_INT: u8 = 0;
pub const CONST_FLOAT: u8 = 1;
pub const CONST_STR: u8 = 2;
pub const CONST_BYTES: u8 = 3;

/// Upvalue descriptor tags.
pub const UPVAL_LOCAL: u8 = 0;
pub const UPVAL_PARENT: u8 = 1;

/// Prototype flag bits.
pub const PROTO_FLAG_VARIADIC: u8 = 0x01;

/// Serialize a module into a fresh byte vector.
pub fn to_bytes(module: &Module) -> Vec<u8> {
    let mut w = Writer::with_capacity(256 + module.total_instructions() * 4);
    w.raw(&crate::MAGIC);
    w.u16(module.version);
    w.u16(0); // reserved flags
    w.utf8_prefixed(&module.name);

    write_consts(&mut w, &module.consts);

    w.uleb_usize(module.protos.len());
    for proto in &module.protos {
        write_proto(&mut w, proto);
    }

    w.uleb(module.entry as u64);
    w.into_bytes()
}

fn write_consts(w: &mut Writer, consts: &[Const]) {
    w.uleb_usize(consts.len());
    for c in consts {
        match c {
            Const::Int(i) => {
                w.u8(CONST_INT).i64(*i);
            }
            Const::Float(x) => {
                w.u8(CONST_FLOAT).f64(*x);
            }
            Const::Str(s) => {
                w.u8(CONST_STR).utf8_prefixed(s);
            }
            Const::Bytes(b) => {
                w.u8(CONST_BYTES).bytes_prefixed(b);
            }
        }
    }
}

fn write_proto(w: &mut Writer, proto: &Proto) {
    w.utf8_prefixed(&proto.name);
    w.uleb(proto.arity as u64);
    w.uleb(proto.reg_count as u64);
    w.u8(if proto.is_variadic { PROTO_FLAG_VARIADIC } else { 0 });

    w.uleb_usize(proto.upvals.len());
    for uv in &proto.upvals {
        match uv {
            UpvalDesc::FromLocal(reg) => {
                w.u8(UPVAL_LOCAL).uleb(*reg as u64);
            }
            UpvalDesc::FromUpval(idx) => {
                w.u8(UPVAL_PARENT).uleb(*idx as u64);
            }
        }
    }

    w.uleb_usize(proto.code.len());
    for instr in &proto.code {
        instr.encode(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instr;

    #[test]
    fn header_starts_with_magic() {
        let m = Module::new();
        let bytes = to_bytes(&m);
        assert_eq!(&bytes[0..4], &crate::MAGIC);
    }

    #[test]
    fn roundtrips_through_loader() {
        let mut m = Module::new();
        m.name = "demo".into();
        let k = m.intern_const(Const::Str("hi".into()));
        let mut p = Proto::new("main");
        p.reg_count = 3;
        p.code.push(Instr::LoadConst { dst: 1, k });
        p.code.push(Instr::Return { src: 1 });
        m.add_proto(p);

        let bytes = to_bytes(&m);
        let loaded = crate::loader::load(&bytes).unwrap();
        assert_eq!(loaded, m);
    }
}
