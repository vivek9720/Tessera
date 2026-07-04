//! The Tessera register virtual machine.
//!
//! The VM executes a verified [`Module`]. Each call pushes a [`Frame`] whose
//! registers are a heap-allocated `Box<[Value]>`; the frame's buffer is released
//! the moment the frame returns. Closures capture enclosing locals as *upvalues*
//! that, while the enclosing frame is live, borrow its register slots directly so
//! that reads and writes are shared. When a frame returns, every upvalue that
//! borrows one of its slots must be *closed* — snapshotted into the upvalue cell
//! — before the register buffer is freed. That teardown is
//! [`Vm::close_frame_upvalues`].
//!
//! Register operands are proven in range by [`crate::verifier`] before execution
//! begins, so the interpreter reads and writes registers without re-checking
//! them. Everything that can legitimately go wrong at runtime — calling a
//! non-function, dividing by zero, indexing past a list — is reported as an
//! [`Error`], never a panic.

use std::collections::HashMap;
use std::mem::size_of;

use crate::builtins;
use crate::bytecode::Instr;
use crate::error::{Error, Result};
use crate::heap::{Heap, Object};
use crate::limits;
use crate::module::{Const, Module, UpvalDesc};
use crate::value::{Handle, Value};

/// A single activation record.
struct Frame {
    /// Index of the running prototype in the module's prototype table.
    proto: u32,
    /// The closure being executed. Also mirrored in register 0.
    closure: Handle,
    /// Register file. `regs[0]` is the reserved self slot.
    regs: Box<[Value]>,
    /// Instruction pointer into the prototype's code array.
    ip: usize,
    /// Register in the *caller* that should receive this frame's return value.
    ret_dst: u16,
}

/// Numeric binary operators sharing an integer/float dispatch.
#[derive(Clone, Copy)]
enum NumOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

/// Ordering comparison operators.
#[derive(Clone, Copy)]
enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
}

/// The virtual machine. A `Vm` owns the heap and global environment for one run;
/// dropping it reclaims every object it allocated.
pub struct Vm {
    heap: Heap,
    globals: HashMap<String, Value>,
    frames: Vec<Frame>,
    /// Upvalue cells that still borrow a live register slot.
    open_upvalues: Vec<Handle>,
    /// Materialized constant pool for the module currently running.
    const_cache: Vec<Value>,
    /// Captured program output (from `print`/`println`).
    output: String,
    /// Instructions retired, checked against [`limits::MAX_STEPS`].
    steps: u64,
}

impl Vm {
    pub fn new() -> Vm {
        Vm {
            heap: Heap::new(),
            globals: HashMap::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
            const_cache: Vec::new(),
            output: String::new(),
            steps: 0,
        }
    }

    /// The text written by the program so far.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Take ownership of the accumulated output, clearing the buffer.
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }

    /// Number of heap objects currently allocated (for diagnostics/tests).
    pub fn heap_len(&self) -> usize {
        self.heap.len()
    }

    /// Verify-free entry point: run an already-verified module to completion and
    /// return its result value.
    pub fn run_module(&mut self, module: &Module) -> Result<Value> {
        self.prepare(module)?;
        self.push_entry_frame(module)?;
        self.execute(module)
    }

    // ---- Setup -------------------------------------------------------------

    fn prepare(&mut self, module: &Module) -> Result<()> {
        // Register builtins into the global namespace.
        for (name, id) in builtins::BUILTINS {
            let handle = self.heap.new_native(*id)?;
            self.globals.insert((*name).to_string(), Value::Obj(handle));
        }

        // Materialize each constant once so `LoadConst` is a plain copy.
        self.const_cache.clear();
        self.const_cache.reserve(module.consts.len());
        for c in &module.consts {
            let v = match c {
                Const::Int(i) => Value::Int(*i),
                Const::Float(x) => Value::Float(*x),
                Const::Str(s) => Value::Obj(self.heap.new_string(s.clone())?),
                Const::Bytes(b) => Value::Obj(self.heap.new_bytes(b.clone())?),
            };
            self.const_cache.push(v);
        }
        Ok(())
    }

    fn push_entry_frame(&mut self, module: &Module) -> Result<()> {
        let entry = module.entry as usize;
        let proto = module
            .protos
            .get(entry)
            .ok_or_else(|| Error::runtime("entry prototype out of range"))?;
        if !proto.upvals.is_empty() {
            return Err(Error::runtime("entry function must not declare upvalues"));
        }
        let reg_count = proto.reg_count as usize;
        let closure = self.heap.new_closure(module.entry, Vec::new())?;
        let mut regs = vec![Value::Nil; reg_count].into_boxed_slice();
        regs[0] = Value::Obj(closure);
        self.frames.push(Frame {
            proto: module.entry,
            closure,
            regs,
            ip: 0,
            ret_dst: 0,
        });
        Ok(())
    }

    // ---- Main loop ---------------------------------------------------------

    fn execute(&mut self, module: &Module) -> Result<Value> {
        loop {
            self.steps += 1;
            if self.steps > limits::MAX_STEPS {
                return Err(Error::runtime("execution step limit exceeded"));
            }

            let fi = self.frames.len() - 1;
            let proto_idx = self.frames[fi].proto as usize;
            let ip = self.frames[fi].ip;
            let code_len = module.protos[proto_idx].code.len();

            // Falling off the end of a function is an implicit `return nil`.
            if ip >= code_len {
                if let Some(result) = self.do_return(Value::Nil)? {
                    return Ok(result);
                }
                continue;
            }

            let instr = module.protos[proto_idx].code[ip];
            self.frames[fi].ip = ip + 1;

            match instr {
                Instr::LoadNil { dst } => self.set(fi, dst, Value::Nil),
                Instr::LoadTrue { dst } => self.set(fi, dst, Value::Bool(true)),
                Instr::LoadFalse { dst } => self.set(fi, dst, Value::Bool(false)),
                Instr::LoadConst { dst, k } => {
                    let v = self.const_cache[k as usize];
                    self.set(fi, dst, v);
                }
                Instr::LoadInt { dst, imm } => self.set(fi, dst, Value::Int(imm as i64)),
                Instr::Move { dst, src } => {
                    let v = self.get(fi, src);
                    self.set(fi, dst, v);
                }

                Instr::Add { dst, a, b } => self.num_binop(fi, NumOp::Add, dst, a, b)?,
                Instr::Sub { dst, a, b } => self.num_binop(fi, NumOp::Sub, dst, a, b)?,
                Instr::Mul { dst, a, b } => self.num_binop(fi, NumOp::Mul, dst, a, b)?,
                Instr::Div { dst, a, b } => self.num_binop(fi, NumOp::Div, dst, a, b)?,
                Instr::Mod { dst, a, b } => self.num_binop(fi, NumOp::Mod, dst, a, b)?,
                Instr::Pow { dst, a, b } => self.num_binop(fi, NumOp::Pow, dst, a, b)?,
                Instr::Neg { dst, a } => {
                    let v = self.get(fi, a);
                    let r = match v {
                        Value::Int(i) => Value::Int(i.wrapping_neg()),
                        Value::Float(x) => Value::Float(-x),
                        other => {
                            return Err(Error::type_error(format!(
                                "cannot negate {}",
                                other.value_type().name()
                            )))
                        }
                    };
                    self.set(fi, dst, r);
                }

                Instr::Eq { dst, a, b } => {
                    let (x, y) = (self.get(fi, a), self.get(fi, b));
                    let r = self.heap.values_equal(x, y);
                    self.set(fi, dst, Value::Bool(r));
                }
                Instr::Ne { dst, a, b } => {
                    let (x, y) = (self.get(fi, a), self.get(fi, b));
                    let r = self.heap.values_equal(x, y);
                    self.set(fi, dst, Value::Bool(!r));
                }
                Instr::Lt { dst, a, b } => self.cmp_binop(fi, CmpOp::Lt, dst, a, b)?,
                Instr::Le { dst, a, b } => self.cmp_binop(fi, CmpOp::Le, dst, a, b)?,
                Instr::Gt { dst, a, b } => self.cmp_binop(fi, CmpOp::Gt, dst, a, b)?,
                Instr::Ge { dst, a, b } => self.cmp_binop(fi, CmpOp::Ge, dst, a, b)?,
                Instr::Not { dst, a } => {
                    let v = self.get(fi, a);
                    self.set(fi, dst, Value::Bool(!v.is_truthy()));
                }

                Instr::Jump { target } => self.frames[fi].ip = target as usize,
                Instr::JumpIfFalse { cond, target } => {
                    if !self.get(fi, cond).is_truthy() {
                        self.frames[fi].ip = target as usize;
                    }
                }
                Instr::JumpIfTrue { cond, target } => {
                    if self.get(fi, cond).is_truthy() {
                        self.frames[fi].ip = target as usize;
                    }
                }

                Instr::Call { dst, callee, base, argc } => {
                    let callee_val = self.get(fi, callee);
                    let mut args = Vec::with_capacity(argc as usize);
                    for j in 0..argc as usize {
                        args.push(self.get(fi, base + j as u16));
                    }
                    self.call_value(callee_val, &args, dst, module)?;
                }
                Instr::TailCall { callee, base, argc } => {
                    let callee_val = self.get(fi, callee);
                    let mut args = Vec::with_capacity(argc as usize);
                    for j in 0..argc as usize {
                        args.push(self.get(fi, base + j as u16));
                    }
                    if let Some(result) = self.tail_call(callee_val, &args, module)? {
                        return Ok(result);
                    }
                }
                Instr::Return { src } => {
                    let v = self.get(fi, src);
                    if let Some(result) = self.do_return(v)? {
                        return Ok(result);
                    }
                }

                Instr::MakeList { dst, base, count } => {
                    let handle = self.make_list(fi, base, count)?;
                    self.set(fi, dst, Value::Obj(handle));
                }
                Instr::MakeMap { dst, base, count } => {
                    let mut pairs = Vec::with_capacity(count as usize);
                    for j in 0..count as usize {
                        let k = self.get(fi, base + (2 * j) as u16);
                        let v = self.get(fi, base + (2 * j + 1) as u16);
                        pairs.push((k, v));
                    }
                    let handle = self.heap.new_map(pairs)?;
                    self.set(fi, dst, Value::Obj(handle));
                }
                Instr::Index { dst, obj, key } => {
                    let (o, k) = (self.get(fi, obj), self.get(fi, key));
                    let r = self.index_get(o, k)?;
                    self.set(fi, dst, r);
                }
                Instr::SetIndex { obj, key, val } => {
                    let (o, k, v) = (self.get(fi, obj), self.get(fi, key), self.get(fi, val));
                    self.index_set(o, k, v)?;
                }
                Instr::Len { dst, obj } => {
                    let o = self.get(fi, obj);
                    let n = match o {
                        Value::Obj(h) => self.heap.length_of(h)?,
                        other => {
                            return Err(Error::type_error(format!(
                                "value of type {} has no length",
                                other.value_type().name()
                            )))
                        }
                    };
                    self.set(fi, dst, Value::Int(n));
                }
                Instr::Append { obj, val } => {
                    let (o, v) = (self.get(fi, obj), self.get(fi, val));
                    self.append(o, v)?;
                }
                Instr::Concat { dst, a, b } => {
                    let (x, y) = (self.get(fi, a), self.get(fi, b));
                    let r = self.concat(x, y)?;
                    self.set(fi, dst, r);
                }

                Instr::GetGlobal { dst, name } => {
                    let key = self.const_str(module, name)?;
                    let v = self.globals.get(key).copied().unwrap_or(Value::Nil);
                    self.set(fi, dst, v);
                }
                Instr::SetGlobal { name, src } => {
                    let v = self.get(fi, src);
                    let key = self.const_str(module, name)?.to_owned();
                    self.globals.insert(key, v);
                }
                Instr::GetUpval { dst, uv } => {
                    let cell = self.closure_upvalue(fi, uv)?;
                    let v = self.heap.upvalue_get(cell)?;
                    self.set(fi, dst, v);
                }
                Instr::SetUpval { uv, src } => {
                    let v = self.get(fi, src);
                    let cell = self.closure_upvalue(fi, uv)?;
                    self.heap.upvalue_set(cell, v)?;
                }
                Instr::Closure { dst, proto } => {
                    let handle = self.make_closure(fi, proto, module)?;
                    self.set(fi, dst, Value::Obj(handle));
                }

                Instr::TypeOf { dst, a } => {
                    let v = self.get(fi, a);
                    let name = match v {
                        Value::Obj(h) => self.heap.get(h)?.kind_name(),
                        other => other.value_type().name(),
                    };
                    let handle = self.heap.new_string(name)?;
                    self.set(fi, dst, Value::Obj(handle));
                }
                Instr::Nop => {}
                Instr::Halt => return Ok(Value::Nil),
            }
        }
    }

    // ---- Register access ---------------------------------------------------

    #[inline]
    fn get(&self, fi: usize, r: u16) -> Value {
        self.frames[fi].regs[r as usize]
    }

    #[inline]
    fn set(&mut self, fi: usize, r: u16, v: Value) {
        self.frames[fi].regs[r as usize] = v;
    }

    // ---- Collection construction ------------------------------------------

    /// Build a list from the register window `base .. base + count`.
    ///
    /// The verifier has already checked the destination and base registers, so
    /// the elements are gathered straight from the register file through the
    /// frame's base pointer without re-validating each slot on this hot path.
    fn make_list(&mut self, fi: usize, base: u16, count: u16) -> Result<Handle> {
        let regs_ptr = self.frames[fi].regs.as_ptr();
        let mut items = Vec::with_capacity(count as usize);
        for j in 0..count as usize {
            let slot = base as usize + j;
            let v = unsafe { *regs_ptr.add(slot) };
            items.push(v);
        }
        self.heap.new_list(items)
    }

    // ---- Calls and returns -------------------------------------------------

    fn call_value(
        &mut self,
        callee: Value,
        args: &[Value],
        ret_dst: u16,
        module: &Module,
    ) -> Result<()> {
        let h = match callee {
            Value::Obj(h) => h,
            other => {
                return Err(Error::type_error(format!(
                    "cannot call value of type {}",
                    other.value_type().name()
                )))
            }
        };
        let kind = self.callable_kind(h)?;
        match kind {
            Callable::Native(id) => {
                let fi = self.frames.len() - 1;
                let r = builtins::dispatch(id, &mut self.heap, args, &mut self.output)?;
                self.set(fi, ret_dst, r);
                Ok(())
            }
            Callable::Closure(proto_idx) => {
                self.push_closure_frame(h, proto_idx, args, ret_dst, module)
            }
        }
    }

    /// Perform a tail call, reusing the current frame's return destination.
    /// Returns `Some(result)` if the tail call completed the whole program (a
    /// native tail call from the entry frame).
    fn tail_call(
        &mut self,
        callee: Value,
        args: &[Value],
        module: &Module,
    ) -> Result<Option<Value>> {
        let h = match callee {
            Value::Obj(h) => h,
            other => {
                return Err(Error::type_error(format!(
                    "cannot call value of type {}",
                    other.value_type().name()
                )))
            }
        };
        let kind = self.callable_kind(h)?;

        let fi = self.frames.len() - 1;
        let ret_dst = self.frames[fi].ret_dst;
        // The current frame is about to be discarded; close any upvalues that
        // borrow its registers first.
        self.close_frame_upvalues(fi);
        self.frames.pop();
        let had_caller = !self.frames.is_empty();

        match kind {
            Callable::Native(id) => {
                let r = builtins::dispatch(id, &mut self.heap, args, &mut self.output)?;
                if had_caller {
                    let caller = self.frames.len() - 1;
                    self.set(caller, ret_dst, r);
                    Ok(None)
                } else {
                    Ok(Some(r))
                }
            }
            Callable::Closure(proto_idx) => {
                self.push_closure_frame(h, proto_idx, args, ret_dst, module)?;
                Ok(None)
            }
        }
    }

    fn callable_kind(&self, h: Handle) -> Result<Callable> {
        match self.heap.get(h)? {
            Object::Closure(c) => Ok(Callable::Closure(c.proto)),
            Object::Native(id) => Ok(Callable::Native(*id)),
            other => Err(Error::type_error(format!(
                "cannot call value of type {}",
                other.kind_name()
            ))),
        }
    }

    fn push_closure_frame(
        &mut self,
        closure: Handle,
        proto_idx: u32,
        args: &[Value],
        ret_dst: u16,
        module: &Module,
    ) -> Result<()> {
        if self.frames.len() >= limits::MAX_CALL_DEPTH {
            return Err(Error::runtime("call stack overflow"));
        }
        let proto = module
            .protos
            .get(proto_idx as usize)
            .ok_or_else(|| Error::runtime("closure references an invalid prototype"))?;
        let reg_count = proto.reg_count as usize;
        let arity = proto.arity as usize;

        let mut regs = vec![Value::Nil; reg_count].into_boxed_slice();
        regs[0] = Value::Obj(closure);
        for i in 0..arity {
            regs[1 + i] = args.get(i).copied().unwrap_or(Value::Nil);
        }

        self.frames.push(Frame {
            proto: proto_idx,
            closure,
            regs,
            ip: 0,
            ret_dst,
        });
        Ok(())
    }

    /// Pop the current frame, delivering `val` to its caller. Returns
    /// `Some(val)` when the entry frame returns (the program is done).
    fn do_return(&mut self, val: Value) -> Result<Option<Value>> {
        let fi = self.frames.len() - 1;
        self.close_frame_upvalues(fi);
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| Error::runtime("return with no active frame"))?;

        if self.frames.is_empty() {
            return Ok(Some(val));
        }
        let caller = self.frames.len() - 1;
        let dst = frame.ret_dst as usize;
        self.frames[caller].regs[dst] = val;
        Ok(None)
    }

    // ---- Upvalues ----------------------------------------------------------

    fn closure_upvalue(&self, fi: usize, uv: u16) -> Result<Handle> {
        let closure = self.frames[fi].closure;
        let c = self.heap.as_closure(closure)?;
        c.upvalues
            .get(uv as usize)
            .copied()
            .ok_or_else(|| Error::runtime("upvalue index out of range"))
    }

    fn make_closure(&mut self, fi: usize, proto_idx: u32, module: &Module) -> Result<Handle> {
        let descs = match module.protos.get(proto_idx as usize) {
            Some(p) => p.upvals.clone(),
            None => return Err(Error::runtime("closure prototype out of range")),
        };
        let parent_reg_count = self.frames[fi].regs.len();
        let parent_closure = self.frames[fi].closure;

        let mut upvalues: Vec<Handle> = Vec::with_capacity(descs.len());
        for desc in &descs {
            let cell = match *desc {
                UpvalDesc::FromLocal(reg) => {
                    let reg = reg as usize;
                    if reg >= parent_reg_count {
                        return Err(Error::runtime("upvalue captures an out-of-range register"));
                    }
                    let loc = unsafe { self.frames[fi].regs.as_mut_ptr().add(reg) };
                    self.find_or_create_open_upvalue(loc)?
                }
                UpvalDesc::FromUpval(idx) => {
                    let parent = self.heap.as_closure(parent_closure)?;
                    *parent
                        .upvalues
                        .get(idx as usize)
                        .ok_or_else(|| Error::runtime("parent upvalue index out of range"))?
                }
            };
            upvalues.push(cell);
        }
        self.heap.new_closure(proto_idx, upvalues)
    }

    fn find_or_create_open_upvalue(&mut self, loc: *mut Value) -> Result<Handle> {
        let addr = loc as usize;
        for &cell in &self.open_upvalues {
            if self.heap.upvalue_addr(cell) == Some(addr) {
                return Ok(cell);
            }
        }
        let cell = self.heap.new_open_upvalue(loc)?;
        self.open_upvalues.push(cell);
        Ok(cell)
    }

    /// Close every open upvalue that borrows a register slot of frame `fi`,
    /// snapshotting the current value into each cell so it survives the frame's
    /// register buffer being freed.
    fn close_frame_upvalues(&mut self, fi: usize) {
        let (lo, count) = {
            let regs = &self.frames[fi].regs;
            (regs.as_ptr() as usize, regs.len())
        };
        // One past the frame's last register slot: the exclusive upper bound of
        // the address range owned by this frame.
        let hi = lo.wrapping_add(count.saturating_sub(1) * size_of::<Value>());

        let mut i = 0;
        while i < self.open_upvalues.len() {
            let cell = self.open_upvalues[i];
            match self.heap.upvalue_addr(cell) {
                Some(addr) if addr >= lo && addr < hi => {
                    self.heap.upvalue_close(cell);
                    self.open_upvalues.swap_remove(i);
                }
                _ => i += 1,
            }
        }
    }

    // ---- Operators ---------------------------------------------------------

    fn num_binop(&mut self, fi: usize, op: NumOp, dst: u16, a: u16, b: u16) -> Result<()> {
        let (x, y) = (self.get(fi, a), self.get(fi, b));
        let r = match (x, y) {
            (Value::Int(i), Value::Int(j)) if !matches!(op, NumOp::Pow) => {
                Value::Int(int_op(op, i, j)?)
            }
            _ => {
                let i = x.as_number().ok_or_else(|| {
                    Error::type_error(format!("arithmetic on {}", x.value_type().name()))
                })?;
                let j = y.as_number().ok_or_else(|| {
                    Error::type_error(format!("arithmetic on {}", y.value_type().name()))
                })?;
                Value::Float(float_op(op, i, j))
            }
        };
        self.set(fi, dst, r);
        Ok(())
    }

    fn cmp_binop(&mut self, fi: usize, op: CmpOp, dst: u16, a: u16, b: u16) -> Result<()> {
        let (x, y) = (self.get(fi, a), self.get(fi, b));
        let ordering = self.compare(x, y)?;
        let r = match op {
            CmpOp::Lt => ordering == std::cmp::Ordering::Less,
            CmpOp::Le => ordering != std::cmp::Ordering::Greater,
            CmpOp::Gt => ordering == std::cmp::Ordering::Greater,
            CmpOp::Ge => ordering != std::cmp::Ordering::Less,
        };
        self.set(fi, dst, Value::Bool(r));
        Ok(())
    }

    fn compare(&self, x: Value, y: Value) -> Result<std::cmp::Ordering> {
        use std::cmp::Ordering;
        match (x, y) {
            (Value::Int(a), Value::Int(b)) => Ok(a.cmp(&b)),
            (Value::Obj(ha), Value::Obj(hb)) => {
                match (self.heap.get(ha)?, self.heap.get(hb)?) {
                    (Object::Str(a), Object::Str(b)) => Ok(a.cmp(b)),
                    _ => Err(Error::type_error("values are not comparable")),
                }
            }
            _ => {
                let a = x
                    .as_number()
                    .ok_or_else(|| Error::type_error("values are not comparable"))?;
                let b = y
                    .as_number()
                    .ok_or_else(|| Error::type_error("values are not comparable"))?;
                a.partial_cmp(&b)
                    .ok_or_else(|| Error::runtime("comparison with NaN"))
            }
        }
    }

    fn concat(&mut self, x: Value, y: Value) -> Result<Value> {
        match (x, y) {
            (Value::Obj(ha), Value::Obj(hb)) => {
                enum Kind {
                    Str(String),
                    List(Vec<Value>),
                }
                let kind = match (self.heap.get(ha)?, self.heap.get(hb)?) {
                    (Object::Str(a), Object::Str(b)) => {
                        if a.len() + b.len() > limits::MAX_STRING_LEN {
                            return Err(Error::runtime("concatenated string too large"));
                        }
                        Kind::Str(format!("{a}{b}"))
                    }
                    (Object::List(a), Object::List(b)) => {
                        let mut v = a.clone();
                        v.extend_from_slice(b);
                        if v.len() > limits::MAX_COLLECTION_LEN {
                            return Err(Error::runtime("concatenated list too large"));
                        }
                        Kind::List(v)
                    }
                    _ => return Err(Error::type_error("cannot concatenate these values")),
                };
                let handle = match kind {
                    Kind::Str(s) => self.heap.new_string(s)?,
                    Kind::List(v) => self.heap.new_list(v)?,
                };
                Ok(Value::Obj(handle))
            }
            _ => Err(Error::type_error("concatenation requires two strings or two lists")),
        }
    }

    fn index_get(&mut self, obj: Value, key: Value) -> Result<Value> {
        let h = obj
            .as_handle()
            .ok_or_else(|| Error::type_error("cannot index a scalar value"))?;
        match self.heap.get(h)? {
            Object::List(items) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| Error::type_error("list index must be an integer"))?;
                normalize_index(i, items.len())
                    .and_then(|idx| items.get(idx).copied())
                    .ok_or_else(|| Error::bounds("list index out of range"))
            }
            Object::Map(pairs) => Ok(pairs
                .iter()
                .find(|(k, _)| self.heap.values_equal(*k, key))
                .map(|(_, v)| *v)
                .unwrap_or(Value::Nil)),
            Object::Str(s) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| Error::type_error("string index must be an integer"))?;
                let chars: Vec<char> = s.chars().collect();
                let idx =
                    normalize_index(i, chars.len()).ok_or_else(|| Error::bounds("string index out of range"))?;
                let ch = chars[idx];
                Ok(Value::Obj(self.heap.new_string(ch.to_string())?))
            }
            Object::Bytes(b) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| Error::type_error("bytes index must be an integer"))?;
                let idx =
                    normalize_index(i, b.len()).ok_or_else(|| Error::bounds("bytes index out of range"))?;
                Ok(Value::Int(b[idx] as i64))
            }
            other => Err(Error::type_error(format!(
                "cannot index value of type {}",
                other.kind_name()
            ))),
        }
    }

    fn index_set(&mut self, obj: Value, key: Value, val: Value) -> Result<()> {
        let h = obj
            .as_handle()
            .ok_or_else(|| Error::type_error("cannot index-assign a scalar value"))?;
        // Map assignment needs an equality scan first, which borrows the heap
        // immutably; resolve the target position before taking a mutable borrow.
        let map_pos = if let Object::Map(pairs) = self.heap.get(h)? {
            let mut found = None;
            for (idx, (k, _)) in pairs.iter().enumerate() {
                if self.heap.values_equal(*k, key) {
                    found = Some(idx);
                    break;
                }
            }
            Some(found)
        } else {
            None
        };

        match self.heap.get_mut(h)? {
            Object::List(items) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| Error::type_error("list index must be an integer"))?;
                let idx = normalize_index(i, items.len())
                    .ok_or_else(|| Error::bounds("list index out of range"))?;
                items[idx] = val;
                Ok(())
            }
            Object::Map(pairs) => {
                match map_pos.flatten() {
                    Some(idx) => pairs[idx].1 = val,
                    None => {
                        if pairs.len() >= limits::MAX_COLLECTION_LEN {
                            return Err(Error::runtime("map too large"));
                        }
                        pairs.push((key, val));
                    }
                }
                Ok(())
            }
            other => Err(Error::type_error(format!(
                "cannot index-assign value of type {}",
                other.kind_name()
            ))),
        }
    }

    fn append(&mut self, obj: Value, val: Value) -> Result<()> {
        let h = obj
            .as_handle()
            .ok_or_else(|| Error::type_error("append expects a list"))?;
        match self.heap.get_mut(h)? {
            Object::List(items) => {
                if items.len() >= limits::MAX_COLLECTION_LEN {
                    return Err(Error::runtime("list too large"));
                }
                items.push(val);
                Ok(())
            }
            other => Err(Error::type_error(format!(
                "append expects a list, got {}",
                other.kind_name()
            ))),
        }
    }

    fn const_str<'m>(&self, module: &'m Module, k: u32) -> Result<&'m str> {
        match module.consts.get(k as usize) {
            Some(Const::Str(s)) => Ok(s),
            _ => Err(Error::runtime("global name constant is not a string")),
        }
    }
}

impl Default for Vm {
    fn default() -> Vm {
        Vm::new()
    }
}

/// The resolved callable kind for a handle.
enum Callable {
    Closure(u32),
    Native(u32),
}

/// Integer arithmetic with wrapping semantics and checked division.
fn int_op(op: NumOp, a: i64, b: i64) -> Result<i64> {
    let v = match op {
        NumOp::Add => a.wrapping_add(b),
        NumOp::Sub => a.wrapping_sub(b),
        NumOp::Mul => a.wrapping_mul(b),
        NumOp::Div => {
            if b == 0 {
                return Err(Error::runtime("integer division by zero"));
            }
            a.wrapping_div(b)
        }
        NumOp::Mod => {
            if b == 0 {
                return Err(Error::runtime("integer modulo by zero"));
            }
            a.wrapping_rem(b)
        }
        NumOp::Pow => unreachable!("pow is handled on the float path"),
    };
    Ok(v)
}

/// Floating-point arithmetic; division and modulo by zero follow IEEE-754.
fn float_op(op: NumOp, a: f64, b: f64) -> f64 {
    match op {
        NumOp::Add => a + b,
        NumOp::Sub => a - b,
        NumOp::Mul => a * b,
        NumOp::Div => a / b,
        NumOp::Mod => a % b,
        NumOp::Pow => a.powf(b),
    }
}

/// Convert a possibly-negative index into a bounds-checked forward index.
/// Negative indices count from the end, matching the surface language.
fn normalize_index(i: i64, len: usize) -> Option<usize> {
    let idx = if i < 0 { i + len as i64 } else { i };
    if idx < 0 || idx as u128 >= len as u128 {
        None
    } else {
        Some(idx as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instr;
    use crate::module::{Module, Proto};

    fn run(module: &Module) -> Result<Value> {
        crate::verifier::verify(module)?;
        let mut vm = Vm::new();
        vm.run_module(module)
    }

    #[test]
    fn arithmetic_and_return() {
        let mut m = Module::new();
        let mut p = Proto::new("main");
        p.reg_count = 4;
        p.code = vec![
            Instr::LoadInt { dst: 1, imm: 20 },
            Instr::LoadInt { dst: 2, imm: 22 },
            Instr::Add { dst: 3, a: 1, b: 2 },
            Instr::Return { src: 3 },
        ];
        m.add_proto(p);
        assert_eq!(run(&m).unwrap().as_int(), Some(42));
    }

    #[test]
    fn division_by_zero_is_error_not_panic() {
        let mut m = Module::new();
        let mut p = Proto::new("main");
        p.reg_count = 4;
        p.code = vec![
            Instr::LoadInt { dst: 1, imm: 1 },
            Instr::LoadInt { dst: 2, imm: 0 },
            Instr::Div { dst: 3, a: 1, b: 2 },
            Instr::Return { src: 3 },
        ];
        m.add_proto(p);
        assert!(run(&m).is_err());
    }

    #[test]
    fn list_build_and_len() {
        let mut m = Module::new();
        let mut p = Proto::new("main");
        p.reg_count = 5;
        p.code = vec![
            Instr::LoadInt { dst: 1, imm: 10 },
            Instr::LoadInt { dst: 2, imm: 20 },
            Instr::LoadInt { dst: 3, imm: 30 },
            Instr::MakeList { dst: 4, base: 1, count: 3 },
            Instr::Len { dst: 1, obj: 4 },
            Instr::Return { src: 1 },
        ];
        m.add_proto(p);
        assert_eq!(run(&m).unwrap().as_int(), Some(3));
    }

    #[test]
    fn closure_captures_and_reads_upvalue() {
        // outer(): local x=7 at reg 1; inner captures x; outer returns inner();
        let mut m = Module::new();

        // inner: returns upvalue 0
        let mut inner = Proto::new("inner");
        inner.reg_count = 2;
        inner.upvals = vec![UpvalDesc::FromLocal(1)];
        inner.code = vec![Instr::GetUpval { dst: 1, uv: 0 }, Instr::Return { src: 1 }];
        let inner_idx = m.add_proto(inner);

        // outer: x=7 in reg1, make closure of inner in reg2, call it into reg3
        let mut outer = Proto::new("main");
        outer.reg_count = 4;
        outer.code = vec![
            Instr::LoadInt { dst: 1, imm: 7 },
            Instr::Closure { dst: 2, proto: inner_idx },
            Instr::Call { dst: 3, callee: 2, base: 3, argc: 0 },
            Instr::Return { src: 3 },
        ];
        m.entry = m.add_proto(outer);

        assert_eq!(run(&m).unwrap().as_int(), Some(7));
    }

    #[test]
    fn calling_a_non_function_is_error() {
        let mut m = Module::new();
        let mut p = Proto::new("main");
        p.reg_count = 3;
        p.code = vec![
            Instr::LoadInt { dst: 1, imm: 5 },
            Instr::Call { dst: 2, callee: 1, base: 2, argc: 0 },
            Instr::Return { src: 2 },
        ];
        m.add_proto(p);
        assert!(run(&m).is_err());
    }
}
