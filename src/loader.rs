//! Decoding the Tessera binary module format into an in-memory [`Module`].
//!
//! The loader is the first thing that touches untrusted bytes. It performs only
//! *structural* decoding and coarse size checks (magic, version, counts within
//! [`crate::limits`]); the semantic pass that range-checks every operand against
//! its owning prototype lives in [`crate::verifier`]. Splitting the two keeps
//! each pass small and makes it clear which errors are "malformed bytes" versus
//! "well-formed but invalid program".
//!
//! Every read goes through the bounds-checked [`Reader`], so no input can drive
//! the loader out of range or into a panic; malformed input always returns an
//! `Err`.

use crate::bytecode::Instr;
use crate::error::{Error, Result};
use crate::limits;
use crate::module::{Const, Module, Proto, UpvalDesc};
use crate::reader::Reader;
use crate::serialize::{
    CONST_BYTES, CONST_FLOAT, CONST_INT, CONST_STR, PROTO_FLAG_VARIADIC, UPVAL_LOCAL, UPVAL_PARENT,
};

/// Decode a module from bytes without running the semantic verifier.
pub fn load(bytes: &[u8]) -> Result<Module> {
    let mut r = Reader::new(bytes);
    read_header_magic(&mut r)?;

    let version = r.u16()?;
    if version == 0 || version > crate::MODULE_VERSION {
        return Err(Error::decode(format!(
            "unsupported module version {version} (this build supports 1..={})",
            crate::MODULE_VERSION
        )));
    }
    let _flags = r.u16()?; // reserved; ignored for forward compatibility

    let name = r.utf8_prefixed()?;
    let consts = read_consts(&mut r)?;
    let protos = read_protos(&mut r)?;

    let entry = r.uleb()?;
    let entry = u32::try_from(entry).map_err(|_| Error::decode("entry index out of range"))?;

    r.expect_eof()?;

    let module = Module { version, name, consts, protos, entry };
    Ok(module)
}

fn read_header_magic(r: &mut Reader) -> Result<()> {
    let magic = r.take(4)?;
    if magic != crate::MAGIC {
        return Err(Error::decode("bad magic: not a Tessera module"));
    }
    Ok(())
}

fn read_consts(r: &mut Reader) -> Result<Vec<Const>> {
    let count = r.uleb_usize()?;
    if count > limits::MAX_CONSTS {
        return Err(Error::decode(format!(
            "constant pool too large: {count} > {}",
            limits::MAX_CONSTS
        )));
    }
    let mut consts = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let tag = r.u8()?;
        let c = match tag {
            CONST_INT => Const::Int(r.i64()?),
            CONST_FLOAT => Const::Float(r.f64()?),
            CONST_STR => Const::Str(r.utf8_prefixed()?),
            CONST_BYTES => Const::Bytes(r.bytes_prefixed()?.to_vec()),
            other => {
                return Err(Error::decode(format!("unknown constant tag {other}")));
            }
        };
        consts.push(c);
    }
    Ok(consts)
}

fn read_protos(r: &mut Reader) -> Result<Vec<Proto>> {
    let count = r.uleb_usize()?;
    if count > limits::MAX_PROTOS {
        return Err(Error::decode(format!(
            "too many prototypes: {count} > {}",
            limits::MAX_PROTOS
        )));
    }
    let mut protos = Vec::with_capacity(count.min(1024));
    for i in 0..count {
        protos.push(read_proto(r, i)?);
    }
    Ok(protos)
}

fn read_proto(r: &mut Reader, index: usize) -> Result<Proto> {
    let name = r.utf8_prefixed()?;

    let arity = read_u16(r, "arity")?;
    let reg_count = read_u16(r, "reg_count")?;
    if reg_count == 0 {
        return Err(Error::decode(format!(
            "prototype #{index} declares zero registers (slot 0 is reserved)"
        )));
    }
    if reg_count > limits::MAX_REGISTERS {
        return Err(Error::decode(format!(
            "prototype #{index} register count {reg_count} exceeds limit {}",
            limits::MAX_REGISTERS
        )));
    }

    let flags = r.u8()?;
    let is_variadic = flags & PROTO_FLAG_VARIADIC != 0;

    let upvals = read_upvals(r, index)?;
    let code = read_code(r, index)?;

    Ok(Proto {
        name,
        arity,
        reg_count,
        is_variadic,
        code,
        upvals,
    })
}

fn read_upvals(r: &mut Reader, index: usize) -> Result<Vec<UpvalDesc>> {
    let count = r.uleb_usize()?;
    if count > limits::MAX_UPVALUES {
        return Err(Error::decode(format!(
            "prototype #{index} declares {count} upvalues, over the limit {}",
            limits::MAX_UPVALUES
        )));
    }
    let mut upvals = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = r.u8()?;
        let raw = r.uleb()?;
        let desc = match tag {
            UPVAL_LOCAL => {
                let reg = u16::try_from(raw)
                    .map_err(|_| Error::decode("upvalue local register out of range"))?;
                UpvalDesc::FromLocal(reg)
            }
            UPVAL_PARENT => {
                let idx = u16::try_from(raw)
                    .map_err(|_| Error::decode("parent upvalue index out of range"))?;
                UpvalDesc::FromUpval(idx)
            }
            other => {
                return Err(Error::decode(format!("unknown upvalue tag {other}")));
            }
        };
        upvals.push(desc);
    }
    Ok(upvals)
}

fn read_code(r: &mut Reader, index: usize) -> Result<Vec<Instr>> {
    let count = r.uleb_usize()?;
    if count > limits::MAX_CODE_LEN {
        return Err(Error::decode(format!(
            "prototype #{index} code length {count} exceeds limit {}",
            limits::MAX_CODE_LEN
        )));
    }
    let mut code = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        code.push(Instr::decode(r)?);
    }
    Ok(code)
}

fn read_u16(r: &mut Reader, field: &str) -> Result<u16> {
    let v = r.uleb()?;
    u16::try_from(v).map_err(|_| Error::decode(format!("{field} value out of range")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_input() {
        assert!(load(&[]).is_err());
        assert!(load(b"TS").is_err());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = vec![b'X', b'X', b'X', b'X'];
        bytes.extend_from_slice(&4u16.to_le_bytes());
        assert!(load(&bytes).is_err());
    }

    #[test]
    fn rejects_zero_version() {
        let mut bytes = crate::MAGIC.to_vec();
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        assert!(load(&bytes).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let m = Module::new();
        let mut bytes = crate::serialize::to_bytes(&m);
        bytes.push(0xAA);
        assert!(load(&bytes).is_err());
    }
}
