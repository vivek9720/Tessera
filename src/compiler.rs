//! Lowering the AST to a bytecode [`Module`].
//!
//! The compiler is a single pass over the AST that emits register-machine
//! instructions. Register allocation follows the usual stack discipline: slot 0
//! is the reserved self slot, parameters and locals occupy the low registers of
//! a function, and expression temporaries are pushed above them and released
//! when the surrounding expression finishes. Closures resolve free variables to
//! *upvalues* with the standard recursive walk over the enclosing function
//! stack (`resolve_upvalue`), capturing an enclosing local by its register or
//! chaining through an enclosing upvalue.
//!
//! Everything this pass emits is within the ranges the [`crate::verifier`]
//! enforces: register operands stay below the function's high-water mark, jump
//! targets always land on a real instruction, and gather windows for calls and
//! collection literals are built from freshly allocated contiguous temporaries.

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, LogicOp, Program, Stmt, StmtKind, UnOp};
use crate::bytecode::{Instr, Reg};
use crate::error::{Error, Result, Span};
use crate::limits;
use crate::module::{Const, Module, Proto, UpvalDesc};

/// Compile a parsed program into a module named `name`.
pub fn compile(program: &Program, name: &str) -> Result<Module> {
    let mut c = Compiler {
        module: Module::new(),
        funcs: Vec::new(),
    };
    c.module.name = name.to_string();
    c.push_func("main", &[], Span::synthetic())?;
    for stmt in &program.body {
        c.compile_stmt(stmt)?;
    }
    let main = c.pop_func()?;
    let entry = c.module.add_proto(main);
    c.module.entry = entry;
    Ok(c.module)
}

/// A local variable binding.
struct Local {
    name: String,
    reg: Reg,
    depth: u32,
}

/// A loop being compiled, tracking where `break`/`continue` jump.
struct LoopCtx {
    continue_target: usize,
    breaks: Vec<usize>,
}

/// Per-function compilation state.
struct FuncState {
    name: String,
    params: u16,
    code: Vec<Instr>,
    locals: Vec<Local>,
    scope_depth: u32,
    free_reg: Reg,
    max_reg: Reg,
    upvals: Vec<UpvalDesc>,
    upval_names: Vec<String>,
    loops: Vec<LoopCtx>,
}

struct Compiler {
    module: Module,
    funcs: Vec<FuncState>,
}

impl Compiler {
    // ---- Function stack ----------------------------------------------------

    fn push_func(&mut self, name: &str, params: &[String], span: Span) -> Result<()> {
        if params.len() >= limits::MAX_REGISTERS as usize - 1 {
            return Err(Error::compile(span, "too many parameters"));
        }
        let mut locals = Vec::with_capacity(params.len());
        for (i, p) in params.iter().enumerate() {
            locals.push(Local {
                name: p.clone(),
                reg: (i + 1) as Reg, // slot 0 is reserved
                depth: 0,
            });
        }
        let base = (params.len() + 1) as Reg;
        self.funcs.push(FuncState {
            name: name.to_string(),
            params: params.len() as u16,
            code: Vec::new(),
            locals,
            scope_depth: 0,
            free_reg: base,
            max_reg: base,
            upvals: Vec::new(),
            upval_names: Vec::new(),
            loops: Vec::new(),
        });
        Ok(())
    }

    fn pop_func(&mut self) -> Result<Proto> {
        // Guarantee a terminator and a non-empty code array. Register 0 is safe
        // to clobber here because no later instruction runs.
        self.emit(Instr::LoadNil { dst: 0 });
        self.emit(Instr::Return { src: 0 });

        let fs = self.funcs.pop().expect("function stack underflow");
        if fs.code.len() > limits::MAX_CODE_LEN {
            return Err(Error::compile(
                Span::synthetic(),
                "function body too large",
            ));
        }
        Ok(Proto {
            name: fs.name,
            arity: fs.params,
            reg_count: fs.max_reg,
            is_variadic: false,
            code: fs.code,
            upvals: fs.upvals,
        })
    }

    fn cur(&self) -> &FuncState {
        self.funcs.last().expect("no active function")
    }

    fn cur_mut(&mut self) -> &mut FuncState {
        self.funcs.last_mut().expect("no active function")
    }

    fn cur_level(&self) -> usize {
        self.funcs.len() - 1
    }

    // ---- Registers ---------------------------------------------------------

    /// The first register available for temporaries: one past the reserved slot
    /// and all currently active locals.
    fn floor(&self) -> Reg {
        (1 + self.cur().locals.len()) as Reg
    }

    fn alloc_temp(&mut self) -> Result<Reg> {
        let r = self.cur().free_reg;
        if (r as usize) + 1 >= limits::MAX_REGISTERS as usize {
            return Err(Error::compile(
                Span::synthetic(),
                "function uses too many registers",
            ));
        }
        let fs = self.cur_mut();
        fs.free_reg = r + 1;
        if fs.free_reg > fs.max_reg {
            fs.max_reg = fs.free_reg;
        }
        Ok(r)
    }

    fn reset_temps(&mut self) {
        let floor = self.floor();
        self.cur_mut().free_reg = floor;
    }

    fn add_local(&mut self, name: String, reg: Reg) {
        let depth = self.cur().scope_depth;
        self.cur_mut().locals.push(Local { name, reg, depth });
    }

    fn begin_scope(&mut self) {
        self.cur_mut().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        let depth = self.cur().scope_depth;
        while self
            .cur()
            .locals
            .last()
            .map_or(false, |l| l.depth >= depth)
        {
            self.cur_mut().locals.pop();
        }
        self.cur_mut().scope_depth -= 1;
        self.reset_temps();
    }

    // ---- Emit / jumps ------------------------------------------------------

    fn emit(&mut self, instr: Instr) -> usize {
        let code = &mut self.cur_mut().code;
        code.push(instr);
        code.len() - 1
    }

    fn patch_jump(&mut self, at: usize) {
        let target = self.cur().code.len() as u32;
        if let Some(instr) = self.cur_mut().code.get_mut(at) {
            match instr {
                Instr::Jump { target: t }
                | Instr::JumpIfFalse { target: t, .. }
                | Instr::JumpIfTrue { target: t, .. } => *t = target,
                _ => {}
            }
        }
    }

    // ---- Name resolution ---------------------------------------------------

    fn resolve_local(&self, level: usize, name: &str) -> Option<Reg> {
        self.funcs[level]
            .locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.reg)
    }

    fn resolve_upvalue(&mut self, level: usize, name: &str) -> Result<Option<u16>> {
        if level == 0 {
            return Ok(None);
        }
        // Already captured here?
        if let Some(i) = self.funcs[level].upval_names.iter().position(|n| n == name) {
            return Ok(Some(i as u16));
        }
        let enclosing = level - 1;
        if let Some(reg) = self.resolve_local(enclosing, name) {
            return Ok(Some(self.add_upvalue(level, name, UpvalDesc::FromLocal(reg))?));
        }
        if let Some(parent_idx) = self.resolve_upvalue(enclosing, name)? {
            return Ok(Some(
                self.add_upvalue(level, name, UpvalDesc::FromUpval(parent_idx))?,
            ));
        }
        Ok(None)
    }

    fn add_upvalue(&mut self, level: usize, name: &str, desc: UpvalDesc) -> Result<u16> {
        let fs = &mut self.funcs[level];
        if fs.upvals.len() >= limits::MAX_UPVALUES {
            return Err(Error::compile(
                Span::synthetic(),
                "closure captures too many variables",
            ));
        }
        let idx = fs.upvals.len() as u16;
        fs.upvals.push(desc);
        fs.upval_names.push(name.to_string());
        Ok(idx)
    }

    fn intern_str(&mut self, s: &str) -> u32 {
        self.module.intern_const(Const::Str(s.to_string()))
    }

    // ---- Statements --------------------------------------------------------

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        match &stmt.kind {
            StmtKind::Let(name, expr) => {
                let dst = self.alloc_temp()?;
                self.compile_expr_into(expr, dst)?;
                self.add_local(name.clone(), dst);
                self.reset_temps();
            }
            StmtKind::Func(name, def) => {
                let dst = self.alloc_temp()?;
                // Bind the name before compiling the body so the function can
                // refer to itself recursively (as an upvalue over this slot).
                self.add_local(name.clone(), dst);
                self.compile_func(def, dst)?;
                self.reset_temps();
            }
            StmtKind::Assign(name, expr) => self.compile_assign(name, expr, stmt.span)?,
            StmtKind::AssignIndex(obj, key, val) => {
                let ro = self.alloc_temp()?;
                self.compile_expr_into(obj, ro)?;
                let rk = self.alloc_temp()?;
                self.compile_expr_into(key, rk)?;
                let rv = self.alloc_temp()?;
                self.compile_expr_into(val, rv)?;
                self.emit(Instr::SetIndex { obj: ro, key: rk, val: rv });
                self.reset_temps();
            }
            StmtKind::ExprStmt(expr) => {
                let t = self.alloc_temp()?;
                self.compile_expr_into(expr, t)?;
                self.reset_temps();
            }
            StmtKind::Return(opt) => {
                let t = self.alloc_temp()?;
                match opt {
                    Some(expr) => self.compile_expr_into(expr, t)?,
                    None => {
                        self.emit(Instr::LoadNil { dst: t });
                    }
                }
                self.emit(Instr::Return { src: t });
                self.reset_temps();
            }
            StmtKind::If(cond, then_block, else_block) => {
                self.compile_if(cond, then_block, else_block)?
            }
            StmtKind::While(cond, body) => self.compile_while(cond, body)?,
            StmtKind::Break => self.compile_break(stmt.span)?,
            StmtKind::Continue => self.compile_continue(stmt.span)?,
        }
        Ok(())
    }

    fn compile_assign(&mut self, name: &str, expr: &Expr, _span: Span) -> Result<()> {
        let level = self.cur_level();
        if let Some(reg) = self.resolve_local(level, name) {
            self.compile_expr_into(expr, reg)?;
        } else if let Some(uv) = self.resolve_upvalue(level, name)? {
            let t = self.alloc_temp()?;
            self.compile_expr_into(expr, t)?;
            self.emit(Instr::SetUpval { uv, src: t });
            self.reset_temps();
        } else {
            let k = self.intern_str(name);
            let t = self.alloc_temp()?;
            self.compile_expr_into(expr, t)?;
            self.emit(Instr::SetGlobal { name: k, src: t });
            self.reset_temps();
        }
        Ok(())
    }

    fn compile_if(&mut self, cond: &Expr, then_block: &[Stmt], else_block: &[Stmt]) -> Result<()> {
        let c = self.alloc_temp()?;
        self.compile_expr_into(cond, c)?;
        let jf = self.emit(Instr::JumpIfFalse { cond: c, target: 0 });
        self.reset_temps();

        self.begin_scope();
        for s in then_block {
            self.compile_stmt(s)?;
        }
        self.end_scope();

        if else_block.is_empty() {
            self.patch_jump(jf);
        } else {
            let jend = self.emit(Instr::Jump { target: 0 });
            self.patch_jump(jf);
            self.begin_scope();
            for s in else_block {
                self.compile_stmt(s)?;
            }
            self.end_scope();
            self.patch_jump(jend);
        }
        Ok(())
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) -> Result<()> {
        let loop_start = self.cur().code.len();
        let c = self.alloc_temp()?;
        self.compile_expr_into(cond, c)?;
        let exit = self.emit(Instr::JumpIfFalse { cond: c, target: 0 });
        self.reset_temps();

        self.cur_mut().loops.push(LoopCtx {
            continue_target: loop_start,
            breaks: Vec::new(),
        });
        self.begin_scope();
        for s in body {
            self.compile_stmt(s)?;
        }
        self.end_scope();
        self.emit(Instr::Jump { target: loop_start as u32 });
        self.patch_jump(exit);

        let ctx = self.cur_mut().loops.pop().expect("loop context");
        for b in ctx.breaks {
            self.patch_jump(b);
        }
        Ok(())
    }

    fn compile_break(&mut self, span: Span) -> Result<()> {
        if self.cur().loops.is_empty() {
            return Err(Error::compile(span, "`break` outside of a loop"));
        }
        let at = self.emit(Instr::Jump { target: 0 });
        self.cur_mut().loops.last_mut().unwrap().breaks.push(at);
        Ok(())
    }

    fn compile_continue(&mut self, span: Span) -> Result<()> {
        let target = match self.cur().loops.last() {
            Some(l) => l.continue_target as u32,
            None => return Err(Error::compile(span, "`continue` outside of a loop")),
        };
        self.emit(Instr::Jump { target });
        Ok(())
    }

    // ---- Expressions -------------------------------------------------------

    fn compile_expr_into(&mut self, expr: &Expr, dst: Reg) -> Result<()> {
        let mark = self.cur().free_reg;
        self.compile_expr_inner(expr, dst)?;
        self.cur_mut().free_reg = mark;
        Ok(())
    }

    fn compile_expr_inner(&mut self, expr: &Expr, dst: Reg) -> Result<()> {
        match &expr.kind {
            ExprKind::Nil => {
                self.emit(Instr::LoadNil { dst });
            }
            ExprKind::Bool(true) => {
                self.emit(Instr::LoadTrue { dst });
            }
            ExprKind::Bool(false) => {
                self.emit(Instr::LoadFalse { dst });
            }
            ExprKind::Int(i) => {
                if let Ok(imm) = i32::try_from(*i) {
                    self.emit(Instr::LoadInt { dst, imm });
                } else {
                    let k = self.module.intern_const(Const::Int(*i));
                    self.emit(Instr::LoadConst { dst, k });
                }
            }
            ExprKind::Float(f) => {
                let k = self.module.intern_const(Const::Float(*f));
                self.emit(Instr::LoadConst { dst, k });
            }
            ExprKind::Str(s) => {
                let k = self.module.intern_const(Const::Str(s.clone()));
                self.emit(Instr::LoadConst { dst, k });
            }
            ExprKind::Ident(name) => self.compile_ident(name, dst)?,
            ExprKind::Unary(op, e) => {
                let t = self.alloc_temp()?;
                self.compile_expr_into(e, t)?;
                let instr = match op {
                    UnOp::Neg => Instr::Neg { dst, a: t },
                    UnOp::Not => Instr::Not { dst, a: t },
                };
                self.emit(instr);
            }
            ExprKind::Binary(op, a, b) => {
                let ra = self.alloc_temp()?;
                self.compile_expr_into(a, ra)?;
                let rb = self.alloc_temp()?;
                self.compile_expr_into(b, rb)?;
                self.emit(binary_instr(*op, dst, ra, rb));
            }
            ExprKind::Logical(op, a, b) => self.compile_logical(*op, a, b, dst)?,
            ExprKind::Call(callee, args) => self.compile_call(callee, args, dst, expr.span)?,
            ExprKind::Index(obj, key) => {
                let ro = self.alloc_temp()?;
                self.compile_expr_into(obj, ro)?;
                let rk = self.alloc_temp()?;
                self.compile_expr_into(key, rk)?;
                self.emit(Instr::Index { dst, obj: ro, key: rk });
            }
            ExprKind::List(items) => self.compile_list(items, dst)?,
            ExprKind::Map(pairs) => self.compile_map(pairs, dst)?,
            ExprKind::Func(def) => self.compile_func(def, dst)?,
        }
        Ok(())
    }

    fn compile_ident(&mut self, name: &str, dst: Reg) -> Result<()> {
        let level = self.cur_level();
        if let Some(reg) = self.resolve_local(level, name) {
            if reg != dst {
                self.emit(Instr::Move { dst, src: reg });
            }
        } else if let Some(uv) = self.resolve_upvalue(level, name)? {
            self.emit(Instr::GetUpval { dst, uv });
        } else {
            let k = self.intern_str(name);
            self.emit(Instr::GetGlobal { dst, name: k });
        }
        Ok(())
    }

    fn compile_logical(&mut self, op: LogicOp, a: &Expr, b: &Expr, dst: Reg) -> Result<()> {
        self.compile_expr_into(a, dst)?;
        let jump = match op {
            LogicOp::And => self.emit(Instr::JumpIfFalse { cond: dst, target: 0 }),
            LogicOp::Or => self.emit(Instr::JumpIfTrue { cond: dst, target: 0 }),
        };
        self.compile_expr_into(b, dst)?;
        self.patch_jump(jump);
        Ok(())
    }

    fn compile_call(&mut self, callee: &Expr, args: &[Expr], dst: Reg, span: Span) -> Result<()> {
        if args.len() > u16::MAX as usize {
            return Err(Error::compile(span, "too many call arguments"));
        }
        let callee_reg = self.alloc_temp()?;
        self.compile_expr_into(callee, callee_reg)?;
        let base = self.cur().free_reg;
        for arg in args {
            let r = self.alloc_temp()?;
            self.compile_expr_into(arg, r)?;
        }
        self.emit(Instr::Call {
            dst,
            callee: callee_reg,
            base,
            argc: args.len() as u16,
        });
        Ok(())
    }

    fn compile_list(&mut self, items: &[Expr], dst: Reg) -> Result<()> {
        if items.is_empty() {
            self.emit(Instr::MakeList { dst, base: 0, count: 0 });
            return Ok(());
        }
        if items.len() > u16::MAX as usize {
            return Err(Error::compile(Span::synthetic(), "list literal too large"));
        }
        let base = self.cur().free_reg;
        for item in items {
            let r = self.alloc_temp()?;
            self.compile_expr_into(item, r)?;
        }
        self.emit(Instr::MakeList {
            dst,
            base,
            count: items.len() as u16,
        });
        Ok(())
    }

    fn compile_map(&mut self, pairs: &[(Expr, Expr)], dst: Reg) -> Result<()> {
        if pairs.is_empty() {
            self.emit(Instr::MakeMap { dst, base: 0, count: 0 });
            return Ok(());
        }
        if pairs.len() > (u16::MAX / 2) as usize {
            return Err(Error::compile(Span::synthetic(), "map literal too large"));
        }
        let base = self.cur().free_reg;
        for (k, v) in pairs {
            let rk = self.alloc_temp()?;
            self.compile_expr_into(k, rk)?;
            let rv = self.alloc_temp()?;
            self.compile_expr_into(v, rv)?;
        }
        self.emit(Instr::MakeMap {
            dst,
            base,
            count: pairs.len() as u16,
        });
        Ok(())
    }

    fn compile_func(&mut self, def: &FuncDef, dst: Reg) -> Result<()> {
        let name = def.name.clone().unwrap_or_else(|| "<anonymous>".to_string());
        self.push_func(&name, &def.params, def.span)?;
        for s in &def.body {
            self.compile_stmt(s)?;
        }
        let proto = self.pop_func()?;
        let proto_idx = self.module.add_proto(proto);
        self.emit(Instr::Closure { dst, proto: proto_idx });
        Ok(())
    }
}

fn binary_instr(op: BinOp, dst: Reg, a: Reg, b: Reg) -> Instr {
    match op {
        BinOp::Add => Instr::Add { dst, a, b },
        BinOp::Sub => Instr::Sub { dst, a, b },
        BinOp::Mul => Instr::Mul { dst, a, b },
        BinOp::Div => Instr::Div { dst, a, b },
        BinOp::Mod => Instr::Mod { dst, a, b },
        BinOp::Pow => Instr::Pow { dst, a, b },
        BinOp::Eq => Instr::Eq { dst, a, b },
        BinOp::Ne => Instr::Ne { dst, a, b },
        BinOp::Lt => Instr::Lt { dst, a, b },
        BinOp::Le => Instr::Le { dst, a, b },
        BinOp::Gt => Instr::Gt { dst, a, b },
        BinOp::Ge => Instr::Ge { dst, a, b },
        BinOp::Concat => Instr::Concat { dst, a, b },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn compile_src(src: &str) -> Result<Module> {
        let program = parse(tokenize(src).unwrap())?;
        compile(&program, "test")
    }

    #[test]
    fn compiles_and_verifies_arithmetic() {
        let m = compile_src("let x = 1 + 2 * 3; return x;").unwrap();
        crate::verifier::verify(&m).expect("compiler output must verify");
    }

    #[test]
    fn compiles_closures_that_verify() {
        let src = "func adder(x) { return func(y) { return x + y; }; } let a = adder(3); return a(4);";
        let m = compile_src(src).unwrap();
        crate::verifier::verify(&m).expect("closure output must verify");
        // The inner function must capture `x` as an upvalue over a real local
        // register (never the reserved self slot).
        let inner = m.protos.iter().find(|p| p.name == "<anonymous>").unwrap();
        assert_eq!(inner.upvals.len(), 1);
        assert!(matches!(inner.upvals[0], UpvalDesc::FromLocal(r) if r >= 1));
    }

    #[test]
    fn compiles_control_flow() {
        let src = "let i = 0; let s = 0; while (i < 10) { s = s + i; i = i + 1; } return s;";
        let m = compile_src(src).unwrap();
        crate::verifier::verify(&m).unwrap();
    }

    #[test]
    fn every_jump_target_is_in_range() {
        let src = "if (1 < 2) { return 10; } else { return 20; }";
        let m = compile_src(src).unwrap();
        for proto in &m.protos {
            let n = proto.code.len() as u32;
            for instr in &proto.code {
                match instr {
                    Instr::Jump { target }
                    | Instr::JumpIfFalse { target, .. }
                    | Instr::JumpIfTrue { target, .. } => assert!(*target < n),
                    _ => {}
                }
            }
        }
    }
}
