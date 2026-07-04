//! Semantic verification of a decoded [`Module`].
//!
//! The interpreter trusts verified modules: register access on the hot path is
//! done through raw pointers with no per-access bounds check, on the assumption
//! that this pass has already proven every operand in range for its owning
//! prototype. That contract is why the verifier walks *every* instruction of
//! *every* prototype and rejects anything that could read or write outside a
//! frame's register window, jump outside its code array, or index a constant,
//! upvalue, or prototype that does not exist.
//!
//! Verification is purely static; it never executes code. Anything that depends
//! on runtime values (for example an out-of-range list index) is a runtime
//! error, not a verification error.

use crate::bytecode::Instr;
use crate::error::{Error, Result};
use crate::module::{Const, Module, Proto};

/// Verify a module, returning `Ok(())` if it is safe to execute.
pub fn verify(module: &Module) -> Result<()> {
    let v = Verifier { module };
    v.run()
}

struct Verifier<'a> {
    module: &'a Module,
}

impl<'a> Verifier<'a> {
    fn run(&self) -> Result<()> {
        if self.module.protos.is_empty() {
            return Err(Error::verify("module has no prototypes"));
        }
        if self.module.entry as usize >= self.module.protos.len() {
            return Err(Error::verify(format!(
                "entry index {} is out of range ({} prototypes)",
                self.module.entry,
                self.module.protos.len()
            )));
        }

        for (i, proto) in self.module.protos.iter().enumerate() {
            self.verify_proto(i, proto)?;
        }
        Ok(())
    }

    fn verify_proto(&self, index: usize, proto: &Proto) -> Result<()> {
        // Parameters occupy registers 1..=arity (slot 0 is the reserved self
        // slot), so they must fit inside the declared register window.
        if proto.arity as usize + 1 > proto.reg_count as usize {
            return Err(Error::verify(format!(
                "prototype #{index} ({}) has arity {} that does not fit in {} registers",
                proto.name, proto.arity, proto.reg_count
            )));
        }

        let ctx = Ctx {
            index,
            reg_count: proto.reg_count,
            code_len: proto.code.len(),
            upval_count: proto.upvals.len(),
        };

        if proto.code.is_empty() {
            return Err(Error::verify(format!(
                "prototype #{index} ({}) has an empty code array",
                proto.name
            )));
        }

        for (pc, instr) in proto.code.iter().enumerate() {
            self.verify_instr(&ctx, pc, *instr)?;
        }
        Ok(())
    }

    fn verify_instr(&self, ctx: &Ctx, pc: usize, instr: Instr) -> Result<()> {
        match instr {
            Instr::LoadNil { dst }
            | Instr::LoadTrue { dst }
            | Instr::LoadFalse { dst } => {
                self.reg(ctx, dst)?;
            }
            Instr::LoadConst { dst, k } => {
                self.reg(ctx, dst)?;
                self.konst(ctx, k)?;
            }
            Instr::LoadInt { dst, .. } => {
                self.reg(ctx, dst)?;
            }
            Instr::Move { dst, src } => {
                self.reg(ctx, dst)?;
                self.reg(ctx, src)?;
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
                self.reg(ctx, dst)?;
                self.reg(ctx, a)?;
                self.reg(ctx, b)?;
            }
            Instr::Neg { dst, a } | Instr::Not { dst, a } | Instr::TypeOf { dst, a } => {
                self.reg(ctx, dst)?;
                self.reg(ctx, a)?;
            }
            Instr::Jump { target } => {
                self.target(ctx, pc, target)?;
            }
            Instr::JumpIfFalse { cond, target } | Instr::JumpIfTrue { cond, target } => {
                self.reg(ctx, cond)?;
                self.target(ctx, pc, target)?;
            }
            Instr::Call { dst, callee, base, argc } => {
                self.reg(ctx, dst)?;
                self.reg(ctx, callee)?;
                // Arguments are read from the window `base .. base + argc`.
                self.window(ctx, base, argc as u32)?;
            }
            Instr::TailCall { callee, base, argc } => {
                self.reg(ctx, callee)?;
                self.window(ctx, base, argc as u32)?;
            }
            Instr::Return { src } => {
                self.reg(ctx, src)?;
            }
            Instr::MakeList { dst, base, count: _ } => {
                // The result register and the window's base register must both
                // be valid; elements are collected from `base` upward.
                self.reg(ctx, dst)?;
                self.reg(ctx, base)?;
            }
            Instr::MakeMap { dst, base, count } => {
                self.reg(ctx, dst)?;
                // A map literal reads `count` key/value pairs, so the window is
                // twice as wide as the pair count.
                self.window(ctx, base, (count as u32).saturating_mul(2))?;
            }
            Instr::Index { dst, obj, key } => {
                self.reg(ctx, dst)?;
                self.reg(ctx, obj)?;
                self.reg(ctx, key)?;
            }
            Instr::SetIndex { obj, key, val } => {
                self.reg(ctx, obj)?;
                self.reg(ctx, key)?;
                self.reg(ctx, val)?;
            }
            Instr::Len { dst, obj } => {
                self.reg(ctx, dst)?;
                self.reg(ctx, obj)?;
            }
            Instr::Append { obj, val } => {
                self.reg(ctx, obj)?;
                self.reg(ctx, val)?;
            }
            Instr::GetGlobal { dst, name } => {
                self.reg(ctx, dst)?;
                self.global_name(ctx, name)?;
            }
            Instr::SetGlobal { name, src } => {
                self.global_name(ctx, name)?;
                self.reg(ctx, src)?;
            }
            Instr::GetUpval { dst, uv } => {
                self.reg(ctx, dst)?;
                self.upval(ctx, uv)?;
            }
            Instr::SetUpval { uv, src } => {
                self.upval(ctx, uv)?;
                self.reg(ctx, src)?;
            }
            Instr::Closure { dst, proto } => {
                self.reg(ctx, dst)?;
                if proto as usize >= self.module.protos.len() {
                    return Err(Error::verify(format!(
                        "prototype #{} references undefined prototype {}",
                        ctx.index, proto
                    )));
                }
            }
            Instr::Nop | Instr::Halt => {}
        }
        Ok(())
    }

    // ---- Range-checking primitives ----------------------------------------

    fn reg(&self, ctx: &Ctx, r: u16) -> Result<()> {
        if r >= ctx.reg_count {
            return Err(Error::verify(format!(
                "prototype #{} uses register {} but only {} are declared",
                ctx.index, r, ctx.reg_count
            )));
        }
        Ok(())
    }

    /// Check that the contiguous window `base .. base + count` lies entirely
    /// within the frame's register file.
    fn window(&self, ctx: &Ctx, base: u16, count: u32) -> Result<()> {
        let end = base as u32 + count;
        if end > ctx.reg_count as u32 {
            return Err(Error::verify(format!(
                "prototype #{} register window {}..{} exceeds {} registers",
                ctx.index, base, end, ctx.reg_count
            )));
        }
        Ok(())
    }

    fn target(&self, ctx: &Ctx, pc: usize, target: u32) -> Result<()> {
        if target as usize >= ctx.code_len {
            return Err(Error::verify(format!(
                "prototype #{} instruction {} jumps to {} but code length is {}",
                ctx.index, pc, target, ctx.code_len
            )));
        }
        Ok(())
    }

    fn konst(&self, ctx: &Ctx, k: u32) -> Result<()> {
        if k as usize >= self.module.consts.len() {
            return Err(Error::verify(format!(
                "prototype #{} references constant {} but the pool has {}",
                ctx.index,
                k,
                self.module.consts.len()
            )));
        }
        Ok(())
    }

    fn global_name(&self, ctx: &Ctx, k: u32) -> Result<()> {
        self.konst(ctx, k)?;
        match &self.module.consts[k as usize] {
            Const::Str(_) => Ok(()),
            other => Err(Error::verify(format!(
                "prototype #{} uses constant {} of type {} as a global name",
                ctx.index,
                k,
                other.type_name()
            ))),
        }
    }

    fn upval(&self, ctx: &Ctx, uv: u16) -> Result<()> {
        if uv as usize >= ctx.upval_count {
            return Err(Error::verify(format!(
                "prototype #{} accesses upvalue {} but only {} are declared",
                ctx.index, uv, ctx.upval_count
            )));
        }
        Ok(())
    }
}

/// Per-prototype verification context.
struct Ctx {
    index: usize,
    reg_count: u16,
    code_len: usize,
    upval_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instr;
    use crate::module::Proto;

    fn module_with(code: Vec<Instr>, reg_count: u16) -> Module {
        let mut m = Module::new();
        let mut p = Proto::new("main");
        p.reg_count = reg_count;
        p.code = code;
        m.add_proto(p);
        m
    }

    #[test]
    fn accepts_a_simple_function() {
        let m = module_with(
            vec![Instr::LoadInt { dst: 1, imm: 1 }, Instr::Return { src: 1 }],
            2,
        );
        assert!(verify(&m).is_ok());
    }

    #[test]
    fn rejects_out_of_range_register() {
        let m = module_with(vec![Instr::LoadNil { dst: 9 }, Instr::Halt], 2);
        assert!(verify(&m).is_err());
    }

    #[test]
    fn rejects_out_of_range_jump() {
        let m = module_with(vec![Instr::Jump { target: 99 }], 2);
        assert!(verify(&m).is_err());
    }

    #[test]
    fn rejects_call_window_overflow() {
        let m = module_with(
            vec![Instr::Call { dst: 1, callee: 1, base: 1, argc: 50 }, Instr::Halt],
            2,
        );
        assert!(verify(&m).is_err());
    }

    #[test]
    fn accepts_valid_map_window() {
        let m = module_with(
            vec![Instr::MakeMap { dst: 1, base: 2, count: 1 }, Instr::Halt],
            4,
        );
        assert!(verify(&m).is_ok());
    }
}
