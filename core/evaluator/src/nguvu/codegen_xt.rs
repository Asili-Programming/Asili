//! Xtensa LX6 (ESP32) code generation from IR, through the [`xt32`](super::xt32) encoder: the
//! device target for strict code (`pata jenga --lengo esp32`, see [`super::device`]).
//!
//! The ESP32 has no double-precision unit and 32-bit registers, so every IR value — a 64-bit
//! integer or a double — is a pair of words (low word first): in an allocated register pair
//! ([`TARGET`]) or an 8-byte home slot. Each instruction computes into scratch registers and
//! moves the result to its place, so a result sharing registers with an operand is never read
//! half-written. Doubles and the 64-bit operations the core lacks (multiply, divide, shifts,
//! conversions) call the GCC runtime (`__adddf3`, `__muldi3`, ...); these are IEEE-exact, so
//! results equal native code's.
//!
//! ABI: the windowed one (`entry`/`retw`, calls with `call8`/`callx8`), so firmware C code calls
//! the exports directly. A call's arguments go in `a10`..`a15` (64-bit values in an even-aligned
//! pair), its 64-bit result comes back in `a10:a11`. Direct calls between functions return the
//! status in `a2:a3` and a number in `a4:a5` (`a10:a11` and `a12:a13` at the caller).
//!
//! Every call goes through a literal holding the callee's address (`l32r a8` + `callx8 a8`):
//! ESP-IDF places the runtime in ROM or IRAM, out of `call8` range of flash, and a large program
//! is out of range of itself. Each function's literal pool comes right before its code; the
//! literals carry `R_XTENSA_32` relocations (against the external symbol, or against `.text`
//! with the callee's offset for calls between the program's functions).
//!
//! Frame: `entry a1, F` with the home slots at `sp + 8 * slot`, the direct-call buffer above
//! them, and `F` the locals rounded to 16 bytes plus 32, as GCC lays out a frame that calls.

use super::ir::{Class, FCond, FloatOp, Func, Home, ICond, Inst, IntOp, RtFn, Term, VReg};
use super::regalloc::{allocate_with, Allocation, Loc, Target};
use super::xt32::{self, Asm, BranchCond, Label};

/// The allocator's registers: pairs of `a2`–`a7`, the registers a `call8` preserves (the ESP32
/// calls a helper for every double operation, so values live across calls constantly). Both
/// classes are register pairs here, drawn from the same three (`one_file`), integers trying
/// them from the first and doubles from the last. Home slots are shared by values whose live
/// ranges never overlap (this generator keeps nothing in a slot beyond the value's own life).
/// Constants take no register: each is rebuilt where it is read (`movi`, or `l32r` from the
/// literal pool), which costs no more than a copy and leaves the three pairs to other values.
pub const TARGET: Target = Target {
    int_callee_saved: &[0, 1, 2],
    int_caller_saved: &[],
    float_callee_saved: &[2, 1, 0],
    float_caller_saved: &[],
    share_slots: true,
    one_file: true,
    remat_constants: true,
};

type Pair = (u8, u8);

/// The registers of allocator register `k`.
fn pair(k: u8) -> Pair {
    (2 + 2 * k, 3 + 2 * k)
}

const SP: u8 = 1;
/// Operand and result scratch pairs, in the registers a call clobbers anyway (never allocated).
const S0: Pair = (10, 11);
const S1: Pair = (12, 13);
const S2: Pair = (14, 15);
/// Scratch words (clobbered by calls): addresses, constants, comparison zero.
const T: u8 = 8;
const T2: u8 = 9;
/// First outgoing argument register (`a2` of the callee).
const ARG0: u8 = 10;

/// The runtime functions compiled from IR into the object itself (`device::helper_func`), once
/// each, and called like external functions: their index here names them.
pub const HELPERS: [RtFn; 5] = [
    RtFn::Floor,
    RtFn::Ceil,
    RtFn::FloatToIntSat,
    RtFn::ShiftAmount,
    RtFn::BoardWait,
];

/// Index of `target` in [`HELPERS`].
pub fn helper_index(target: RtFn) -> Option<u32> {
    HELPERS.iter().position(|&h| h == target).map(|i| i as u32)
}

/// Symbol of the C library function implementing `target` on the ESP32. Everything else the
/// device runtime provides on Cortex-M is IR (inlined by `device::inline_runtime`, or a helper
/// in [`HELPERS`]), so the ESP32 links no runtime library of Asili's own.
pub fn runtime_symbol(target: RtFn) -> Option<&'static str> {
    Some(match target {
        RtFn::Fmod => "fmod",
        RtFn::Pow => "pow",
        _ => return None,
    })
}

/// One function's code: its literal pool followed by its instructions.
pub struct Code {
    pub bytes: Vec<u8>,
    /// Offset of the first instruction (the function's address) in `bytes`.
    pub entry: usize,
    /// Direct calls to link: (byte offset in `bytes` of the literal that must hold the callee's
    /// address, callee).
    pub calls: Vec<(usize, u32)>,
    /// Helper calls to link: (literal byte offset, index in [`HELPERS`]).
    pub helpers: Vec<(usize, u32)>,
    /// Bytes of stack the function's own frame takes (`entry a1, frame`).
    pub frame: u32,
    /// `R_XTENSA_32` relocations: (literal byte offset in `bytes`, symbol).
    pub relocs: Vec<(usize, String)>,
}

#[derive(Clone, PartialEq)]
enum Lit {
    Word(u32),
    /// The address of an external function.
    Sym(String),
    /// The address of program function `callee`'s direct (or, for a thunk, ordinary) entry.
    Func(u32),
    /// The address of helper [`HELPERS`]`[i]`.
    Helper(u32),
}

/// A function's literal pool and pending direct calls, shared by functions and export thunks.
#[derive(Default)]
struct Pool {
    lits: Vec<Lit>,
    /// (code offset of the `l32r`, literal index, register)
    uses: Vec<(usize, usize, u8)>,
}

impl Pool {
    fn literal(&mut self, lit: Lit) -> usize {
        if let Some(i) = self.lits.iter().position(|l| *l == lit) {
            return i;
        }
        self.lits.push(lit);
        self.lits.len() - 1
    }

    fn l32r(&mut self, asm: &mut Asm, reg: u8, lit: Lit) {
        let index = self.literal(lit);
        self.uses.push((asm.pc(), index, reg));
        asm.word(0);
    }

    /// `reg = v`
    fn const32(&mut self, asm: &mut Asm, reg: u8, v: u32) -> Result<(), String> {
        let s = v as i32;
        if (-2048..=2047).contains(&s) {
            asm.emit(xt32::movi(reg, s))
        } else {
            self.l32r(asm, reg, Lit::Word(v));
            Ok(())
        }
    }

    /// Call program function `callee` (its address is relocated when the image is laid out).
    fn call_direct(&mut self, asm: &mut Asm, callee: u32) {
        self.l32r(asm, T, Lit::Func(callee));
        asm.word(xt32::callx(8, T).expect("callx8"));
    }

    /// Lays out the literal pool before the code and patches every `l32r`.
    fn finish(self, asm: Asm) -> Result<Code, String> {
        let code = asm.finish()?;
        let pool = 4 * self.lits.len();
        let mut bytes = Vec::with_capacity(pool + code.len());
        let mut relocs = Vec::new();
        let mut calls = Vec::new();
        let mut helpers = Vec::new();
        for (i, lit) in self.lits.iter().enumerate() {
            match lit {
                Lit::Word(w) => bytes.extend_from_slice(&w.to_le_bytes()),
                Lit::Sym(s) => {
                    relocs.push((4 * i, s.clone()));
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                }
                Lit::Func(f) => {
                    calls.push((4 * i, *f));
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                }
                Lit::Helper(h) => {
                    helpers.push((4 * i, *h));
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                }
            }
        }
        bytes.extend_from_slice(&code);
        for &(at, index, reg) in &self.uses {
            let pc = pool + at;
            let w = xt32::l32r_word(reg, xt32::l32r_offset(pc, 4 * index))
                .ok_or("nguvu: hazina ya literal iko mbali mno")?;
            bytes[pc..pc + 3].copy_from_slice(&w.to_le_bytes()[..3]);
        }
        Ok(Code {
            bytes,
            entry: pool,
            calls,
            helpers,
            relocs,
            frame: 0,
        })
    }
}

/// The GCC runtime routine computing `op` on doubles.
fn float_symbol(op: FloatOp) -> &'static str {
    match op {
        FloatOp::Add => "__adddf3",
        FloatOp::Sub => "__subdf3",
        FloatOp::Mul => "__muldf3",
        FloatOp::Div => "__divdf3",
    }
}

/// The libgcc comparison for `cond` and the branch on its result (in `a10`, against `T2` = 0)
/// that means "true": the routines return <0, 0, >0, with unordered operands making each ordered
/// test false (and `__nedf2` non-zero) — exactly the IR's semantics.
fn fcmp_test(cond: FCond) -> (&'static str, BranchCond, u8, u8) {
    match cond {
        FCond::Olt => ("__ltdf2", BranchCond::Lt, ARG0, T2),
        FCond::Ole => ("__ledf2", BranchCond::Ge, T2, ARG0),
        FCond::Ogt => ("__gtdf2", BranchCond::Lt, T2, ARG0),
        FCond::Oge => ("__gedf2", BranchCond::Ge, ARG0, T2),
        FCond::Oeq => ("__eqdf2", BranchCond::Eq, ARG0, T2),
        FCond::Une => ("__nedf2", BranchCond::Ne, ARG0, T2),
    }
}

struct Gen<'f> {
    asm: Asm,
    func: &'f Func,
    alloc: Allocation,
    frame: u32,
    pool: Pool,
    /// The value of each register defined once, by a constant, and not an argument: one left
    /// without a register is rebuilt where it is read instead of kept in a slot.
    consts: Vec<Option<i64>>,
}

/// Xtensa code for `func`; an instruction the device cannot run is an error naming it.
pub fn generate(func: &Func) -> Result<Code, String> {
    let loads = fused_loads(func);
    let mut kept_out = vec![false; func.classes.len()];
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, inst) in block.insts.iter().enumerate() {
            if loads[b][i] {
                kept_out[inst.defs()[0].0 as usize] = true;
            }
        }
    }
    let alloc = allocate_with(func, &TARGET, &kept_out);
    let locals = 8 * alloc.slots + func.call_buffer;
    let frame = locals.div_ceil(16) * 16 + 32;
    if frame > 32760 {
        return Err("kazi ina rejista nyingi mno kwa ESP32".into());
    }
    let mut g = Gen {
        asm: Asm::new(),
        func,
        alloc,
        frame,
        pool: Pool::default(),
        consts: func
            .homes()
            .iter()
            .enumerate()
            .map(|(v, h)| match h {
                Home::Const(bits) if !func.args.contains(&VReg(v as u32)) => Some(*bits),
                _ => None,
            })
            .collect(),
    };
    g.prologue()?;
    let labels: Vec<Label> = (0..func.blocks.len()).map(|_| g.asm.new_label()).collect();
    let order = g.alloc.order.clone();
    for (k, &bi) in order.iter().enumerate() {
        let block = &func.blocks[bi];
        g.asm.bind(labels[bi]);
        let next = order.get(k + 1).map(|&b| labels[b]);
        // A branch on a comparison made last in the block, read nowhere else, branches on the
        // comparison itself instead of a 0/1 value.
        let fused = match (block.insts.last(), &block.term) {
            (Some(last), Term::Branch { cond, .. })
                if matches!(
                    last,
                    Inst::ICmp { .. }
                        | Inst::ICmpImm { .. }
                        | Inst::FCmp { .. }
                        | Inst::TestImm { .. }
                ) && last.defs() == [*cond]
                    && g.alloc.uses[cond.0 as usize] == 1 =>
            {
                Some(last)
            }
            _ => None,
        };
        let body = if fused.is_some() {
            &block.insts[..block.insts.len() - 1]
        } else {
            &block.insts[..]
        };
        for (i, inst) in body.iter().enumerate() {
            if loads[bi][i] {
                continue; // loaded by the double operation that follows (`fused_loads`)
            }
            match (i.checked_sub(1).map(|p| loads[bi][p]), inst) {
                (Some(true), Inst::Float { op, dst, a, b }) => {
                    g.float_loaded(*op, *dst, *a, *b, &body[i - 1])?
                }
                _ => g.inst(inst)?,
            }
        }
        if let (Some(cmp), Term::Branch { then_, else_, .. }) = (fused, &block.term) {
            let (t, e) = (labels[then_.0 as usize], labels[else_.0 as usize]);
            g.branch_on(cmp, t, e, next)?;
            continue;
        }
        match &block.term {
            Term::Jump(t) => {
                if Some(labels[t.0 as usize]) != next {
                    g.asm.jump(labels[t.0 as usize]);
                }
            }
            Term::Branch { cond, then_, else_ } => {
                let (t, e) = (labels[then_.0 as usize], labels[else_.0 as usize]);
                g.nonzero_word(*cond)?; // T = lo | hi, T2 = 0
                                        // Short branches only skip one jump, so blocks may be any distance apart.
                let skip = g.asm.new_label();
                if Some(t) == next {
                    g.asm.branch(BranchCond::Ne, T, T2, skip);
                    g.asm.jump(e);
                    g.asm.bind(skip);
                } else {
                    g.asm.branch(BranchCond::Eq, T, T2, skip);
                    g.asm.jump(t);
                    g.asm.bind(skip);
                    if Some(e) != next {
                        g.asm.jump(e);
                    }
                }
            }
            Term::Return(v) => {
                g.move_into(*v, (2, 3))?;
                g.asm.word(xt32::RETW);
            }
            Term::ReturnNum(v) => {
                g.move_into(*v, (4, 5))?;
                g.emit(xt32::movi(2, 0))?;
                g.emit(xt32::movi(3, 0))?;
                g.asm.word(xt32::RETW);
            }
        }
    }
    let mut code = g.pool.finish(g.asm)?;
    code.frame = frame;
    Ok(code)
}

impl<'f> Gen<'f> {
    fn emit(&mut self, w: Option<u32>) -> Result<(), String> {
        self.asm.emit(w)
    }

    fn slot(&self, v: VReg) -> Result<u32, String> {
        let s = self.alloc.slot[v.0 as usize];
        if s == u32::MAX {
            return Err("nguvu: rejista haina makazi".into());
        }
        Ok(8 * s)
    }

    /// The constant `v` holds, when it has no register and is rebuilt where read.
    fn remat(&self, v: VReg) -> Option<i64> {
        match self.alloc.loc[v.0 as usize] {
            Loc::Slot => self.consts[v.0 as usize],
            _ => None,
        }
    }

    /// The register pair holding `v`, if it was given one.
    fn reg(&self, v: VReg) -> Option<Pair> {
        match self.alloc.loc[v.0 as usize] {
            Loc::Int(k) | Loc::Float(k) => Some(pair(k)),
            Loc::Slot => None,
        }
    }

    fn prologue(&mut self) -> Result<(), String> {
        let frame = self.frame;
        self.emit(xt32::entry(frame))?;
        self.emit(xt32::movi(T2, 0))?;
        // Incoming arguments are pointer-sized, in a2..: those without a register go to their
        // homes (zero-extended); those with one move into it, in an order that overwrites no
        // argument still to be moved (parked in its home and loaded after, in a cycle).
        let args = self.func.args;
        if args.len() > 6 {
            return Err("nguvu: hoja nyingi mno kwa ESP32".into());
        }
        let mut pending: Vec<(u8, Pair)> = Vec::new(); // (incoming register, its pair)
        for (i, &v) in args.iter().enumerate() {
            let incoming = 2 + i as u8;
            match self.reg(v) {
                Some(p) => pending.push((incoming, p)),
                None => {
                    let off = self.slot(v)?;
                    self.st32(incoming, SP, off)?;
                    self.st32(T2, SP, off + 4)?;
                }
            }
        }
        while !pending.is_empty() {
            let free = pending.iter().position(|&(src, p)| {
                pending
                    .iter()
                    .all(|&(other, _)| other == src || (other != p.0 && other != p.1))
            });
            let Some(k) = free else { break };
            let (src, p) = pending.remove(k);
            if p.0 != src {
                self.asm.word(xt32::mov(p.0, src));
            }
            self.emit(xt32::movi(p.1, 0))?;
        }
        // A cycle: park the rest, then load them.
        let parked: Vec<(u8, Pair)> = pending;
        for &(src, _) in &parked {
            let v = args[(src - 2) as usize];
            let off = self.slot(v)?;
            self.st32(src, SP, off)?;
            self.st32(T2, SP, off + 4)?;
        }
        for &(src, p) in &parked {
            let v = args[(src - 2) as usize];
            let off = self.slot(v)?;
            self.ld64(p, SP, off)?;
        }
        Ok(())
    }

    fn l32r(&mut self, reg: u8, lit: Lit) {
        self.pool.l32r(&mut self.asm, reg, lit);
    }

    /// `reg = v`
    fn const32(&mut self, reg: u8, v: u32) -> Result<(), String> {
        self.pool.const32(&mut self.asm, reg, v)
    }

    fn const64(&mut self, p: Pair, v: i64) -> Result<(), String> {
        self.const32(p.0, v as u32)?;
        self.const32(p.1, (v >> 32) as u32)
    }

    /// `[base + off]` as a base register and an offset `l32i`/`s32i` can reach for two words
    /// (forming the address in `T` when `off` is larger).
    fn at(&mut self, base: u8, off: u32) -> Result<(u8, u32), String> {
        if off + 4 <= 1020 {
            Ok((base, off))
        } else {
            self.const32(T, off)?;
            self.asm.word(xt32::add(T, base, T));
            Ok((T, 0))
        }
    }

    fn st32(&mut self, r: u8, base: u8, off: u32) -> Result<(), String> {
        let (b, o) = self.at(base, off)?;
        self.emit(xt32::s32i(r, b, o))
    }

    fn ld64(&mut self, p: Pair, base: u8, off: u32) -> Result<(), String> {
        let (b, o) = self.at(base, off)?;
        if p.0 == b {
            self.emit(xt32::l32i(p.1, b, o + 4))?;
            self.emit(xt32::l32i(p.0, b, o))
        } else {
            self.emit(xt32::l32i(p.0, b, o))?;
            self.emit(xt32::l32i(p.1, b, o + 4))
        }
    }

    fn st64(&mut self, p: Pair, base: u8, off: u32) -> Result<(), String> {
        let (b, o) = self.at(base, off)?;
        self.emit(xt32::s32i(p.0, b, o))?;
        self.emit(xt32::s32i(p.1, b, o + 4))
    }

    fn mov_pair(&mut self, dst: Pair, src: Pair) {
        if dst != src {
            self.asm.word(xt32::mov(dst.0, src.0));
            self.asm.word(xt32::mov(dst.1, src.1));
        }
    }

    // ----- operands -----------------------------------------------------------------------

    /// The pair holding `v`: its register, or `scratch` loaded from its home.
    fn operand(&mut self, v: VReg, scratch: Pair) -> Result<Pair, String> {
        if let Some(c) = self.remat(v) {
            self.const64(scratch, c)?;
            return Ok(scratch);
        }
        match self.reg(v) {
            Some(r) => Ok(r),
            None => {
                let off = self.slot(v)?;
                self.ld64(scratch, SP, off)?;
                Ok(scratch)
            }
        }
    }

    /// `dst = v` for a specific pair (call arguments, return registers).
    fn move_into(&mut self, v: VReg, dst: Pair) -> Result<(), String> {
        if let Some(c) = self.remat(v) {
            return self.const64(dst, c);
        }
        match self.reg(v) {
            Some(r) => {
                self.mov_pair(dst, r);
                Ok(())
            }
            None => {
                let off = self.slot(v)?;
                self.ld64(dst, SP, off)
            }
        }
    }

    /// `r = ` the low word of `v` (a pointer-sized argument).
    fn low_into(&mut self, v: VReg, r: u8) -> Result<(), String> {
        if let Some(c) = self.remat(v) {
            return self.const32(r, c as u32);
        }
        match self.reg(v) {
            Some(p) => {
                if p.0 != r {
                    self.asm.word(xt32::mov(r, p.0));
                }
                Ok(())
            }
            None => {
                let off = self.slot(v)?;
                let (b, o) = self.at(SP, off)?;
                self.emit(xt32::l32i(r, b, o))
            }
        }
    }

    /// The register holding the low word of `v` (an address): its register's, or `scratch`
    /// loaded with it.
    fn address(&mut self, v: VReg, scratch: u8) -> Result<u8, String> {
        match self.reg(v) {
            Some(p) => Ok(p.0),
            None => {
                self.low_into(v, scratch)?;
                Ok(scratch)
            }
        }
    }

    /// Where to compute `v` directly: its register, or `scratch`.
    fn target(&self, v: VReg, scratch: Pair) -> Pair {
        self.reg(v).unwrap_or(scratch)
    }

    /// `v = src`: into its register, or stored to its home.
    fn put(&mut self, v: VReg, src: Pair) -> Result<(), String> {
        match self.reg(v) {
            Some(r) => {
                self.mov_pair(r, src);
                Ok(())
            }
            None => {
                let off = self.slot(v)?;
                self.st64(src, SP, off)
            }
        }
    }

    // ----- calls --------------------------------------------------------------------------

    /// `callx8` the external function `symbol` (arguments already in `a10`..).
    fn call_symbol(&mut self, symbol: &str) {
        self.l32r(T, Lit::Sym(symbol.to_string()));
        self.asm.word(xt32::callx(8, T).expect("callx8"));
    }

    /// `dst = a op b` for a double operation one of whose operands `load` (a fused `Load`,
    /// see [`fused_loads`]) defines: loaded straight into its argument registers.
    fn float_loaded(
        &mut self,
        op: FloatOp,
        dst: VReg,
        a: VReg,
        b: VReg,
        load: &Inst,
    ) -> Result<(), String> {
        let Inst::Load {
            dst: loaded,
            base,
            offset,
        } = *load
        else {
            return Err("nguvu: si upakiaji".into());
        };
        let off = u32::try_from(offset).map_err(|_| "nguvu: anwani hasi")?;
        let (into, other, other_into) = if a == loaded {
            ((ARG0, ARG0 + 1), b, (ARG0 + 2, ARG0 + 3))
        } else {
            ((ARG0 + 2, ARG0 + 3), a, (ARG0, ARG0 + 1))
        };
        // The base in a scratch word the load overwrites last (`ld64`), then the other operand
        // into its own pair, clear of the loaded one.
        let base = self.address(base, T2)?;
        self.ld64(into, base, off)?;
        self.move_into(other, other_into)?;
        self.call_symbol(float_symbol(op));
        self.put(dst, (ARG0, ARG0 + 1))
    }

    /// `dst = symbol(a, b)` on two 64-bit values (doubles or integers), result 64-bit.
    fn call2(&mut self, symbol: &str, dst: VReg, a: VReg, b: VReg) -> Result<(), String> {
        self.move_into(a, (ARG0, ARG0 + 1))?;
        self.move_into(b, (ARG0 + 2, ARG0 + 3))?;
        self.call_symbol(symbol);
        self.put(dst, (ARG0, ARG0 + 1))
    }

    fn call1(&mut self, symbol: &str, dst: VReg, a: VReg) -> Result<(), String> {
        self.move_into(a, (ARG0, ARG0 + 1))?;
        self.call_symbol(symbol);
        self.put(dst, (ARG0, ARG0 + 1))
    }

    // ----- comparisons --------------------------------------------------------------------

    /// `T = lo | hi` of `v`, `T2 = 0`: `v != 0` exactly when `T != T2`.
    fn nonzero_word(&mut self, v: VReg) -> Result<(), String> {
        let p = self.operand(v, S0)?;
        self.asm.word(xt32::or(T, p.0, p.1));
        self.emit(xt32::movi(T2, 0))
    }

    /// `S2 = (x cond y) ? 1 : 0` for 32-bit registers.
    fn bool_branch(&mut self, cond: BranchCond, x: u8, y: u8) -> Result<(), String> {
        let done = self.asm.new_label();
        self.emit(xt32::movi(S2.0, 1))?;
        self.emit(xt32::movi(S2.1, 0))?;
        self.asm.branch(cond, x, y, done);
        self.emit(xt32::movi(S2.0, 0))?;
        self.asm.bind(done);
        Ok(())
    }

    /// `S2 = (a < b) ^ invert` on 64-bit pairs, signed or unsigned.
    fn lt64(&mut self, a: Pair, b: Pair, signed: bool, invert: bool) -> Result<(), String> {
        let (yes, no) = if invert { (0, 1) } else { (1, 0) };
        let done = self.asm.new_label();
        let fals = self.asm.new_label();
        self.emit(xt32::movi(S2.0, yes))?;
        self.emit(xt32::movi(S2.1, 0))?;
        let hi = if signed {
            BranchCond::Lt
        } else {
            BranchCond::Ltu
        };
        self.asm.branch(hi, a.1, b.1, done);
        self.asm.branch(BranchCond::Ne, a.1, b.1, fals);
        self.asm.branch(BranchCond::Ltu, a.0, b.0, done);
        self.asm.bind(fals);
        self.emit(xt32::movi(S2.0, no))?;
        self.asm.bind(done);
        Ok(())
    }

    /// `S2 = (a == b) ^ invert` on 64-bit pairs.
    fn eq64(&mut self, a: Pair, b: Pair, invert: bool) -> Result<(), String> {
        let (yes, no) = if invert { (0, 1) } else { (1, 0) };
        let done = self.asm.new_label();
        self.emit(xt32::movi(S2.0, no))?;
        self.emit(xt32::movi(S2.1, 0))?;
        self.asm.branch(BranchCond::Ne, a.1, b.1, done);
        self.asm.branch(BranchCond::Ne, a.0, b.0, done);
        self.emit(xt32::movi(S2.0, yes))?;
        self.asm.bind(done);
        Ok(())
    }

    /// `S2 = a cond b`.
    fn icmp(&mut self, cond: ICond, a: Pair, b: Pair) -> Result<(), String> {
        match cond {
            ICond::Lt => self.lt64(a, b, true, false),
            ICond::Ge => self.lt64(a, b, true, true),
            ICond::Gt => self.lt64(b, a, true, false),
            ICond::Le => self.lt64(b, a, true, true),
            ICond::Ult => self.lt64(a, b, false, false),
            ICond::Ule => self.lt64(b, a, false, true),
            ICond::Eq => self.eq64(a, b, false),
            ICond::Ne => self.eq64(a, b, true),
        }
    }

    /// Jump to `t` when comparison `cmp` holds, else to `e` (either may be the next block).
    /// Short branches reach local labels only; the jumps to the blocks reach any distance.
    fn branch_on(
        &mut self,
        cmp: &Inst,
        t: Label,
        e: Label,
        next: Option<Label>,
    ) -> Result<(), String> {
        let yes = self.asm.new_label();
        let no = self.asm.new_label();
        // The test falls through to `yes` when only `t` needs a jump (`e` is the next block),
        // to `no` otherwise.
        let fall = if Some(e) == next && Some(t) != next {
            yes
        } else {
            no
        };
        match cmp {
            Inst::ICmp { cond, a, b, .. } => {
                let pa = self.operand(*a, S0)?;
                let pb = self.operand(*b, S1)?;
                self.icmp_branch(*cond, pa, pb, yes, no, fall);
            }
            Inst::ICmpImm { cond, a, imm, .. } => {
                let pa = self.operand(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.icmp_branch(*cond, pa, S1, yes, no, fall);
            }
            Inst::TestImm { zero, a, imm, .. } => {
                let pa = self.operand(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.asm.word(xt32::and(T, pa.0, S1.0));
                self.asm.word(xt32::and(T2, pa.1, S1.1));
                self.asm.word(xt32::or(T, T, T2));
                self.emit(xt32::movi(T2, 0))?;
                let bc = if *zero {
                    BranchCond::Eq
                } else {
                    BranchCond::Ne
                };
                self.last_branch(bc, T, T2, yes, no, fall);
            }
            Inst::FCmp { cond, a, b, .. } => {
                let (symbol, bc, x, y) = fcmp_test(*cond);
                self.move_into(*a, (ARG0, ARG0 + 1))?;
                self.move_into(*b, (ARG0 + 2, ARG0 + 3))?;
                self.call_symbol(symbol);
                self.emit(xt32::movi(T2, 0))?;
                self.last_branch(bc, x, y, yes, no, fall);
            }
            _ => return Err("nguvu: si ulinganisho".into()),
        }
        if fall == yes {
            // Holds: jump to `t`; fails: on into the next block, `e`.
            self.asm.bind(yes);
            self.asm.jump(t);
            self.asm.bind(no);
        } else {
            self.asm.bind(no);
            if Some(e) != next {
                self.asm.jump(e);
            }
            self.asm.bind(yes);
            if Some(t) != next {
                self.asm.jump(t);
            }
        }
        Ok(())
    }

    /// The last test of a comparison: to `t` when `x c y`, else to `f`, where one of them may
    /// be `fall`, bound right after (reached by falling through).
    fn last_branch(&mut self, c: BranchCond, x: u8, y: u8, t: Label, f: Label, fall: Label) {
        if f == fall {
            self.asm.branch(c, x, y, t);
        } else if t == fall {
            self.asm.branch(c.negate(), x, y, f);
        } else {
            self.asm.branch(c, x, y, t);
            self.asm.jump(f);
        }
    }

    /// Branch to `yes` when `a cond b`, else to `no`; `fall` (one of them) is bound right
    /// after, so reaching it means falling through.
    fn icmp_branch(&mut self, cond: ICond, a: Pair, b: Pair, yes: Label, no: Label, fall: Label) {
        // `x < y` on 64-bit pairs: decided by the high words unless they are equal.
        let lt = |g: &mut Self, x: Pair, y: Pair, signed: bool, t: Label, f: Label| {
            let hi = if signed {
                BranchCond::Lt
            } else {
                BranchCond::Ltu
            };
            g.asm.branch(hi, x.1, y.1, t);
            g.asm.branch(BranchCond::Ne, x.1, y.1, f);
            g.last_branch(BranchCond::Ltu, x.0, y.0, t, f, fall);
        };
        // `x == y`: both words equal.
        let eq = |g: &mut Self, x: Pair, y: Pair, t: Label, f: Label| {
            g.asm.branch(BranchCond::Ne, x.1, y.1, f);
            g.last_branch(BranchCond::Eq, x.0, y.0, t, f, fall);
        };
        match cond {
            ICond::Lt => lt(self, a, b, true, yes, no),
            ICond::Ge => lt(self, a, b, true, no, yes),
            ICond::Gt => lt(self, b, a, true, yes, no),
            ICond::Le => lt(self, b, a, true, no, yes),
            ICond::Ult => lt(self, a, b, false, yes, no),
            ICond::Ule => lt(self, b, a, false, no, yes),
            ICond::Eq => eq(self, a, b, yes, no),
            ICond::Ne => eq(self, a, b, no, yes),
        }
    }

    // ----- integer arithmetic -------------------------------------------------------------

    /// `dst = a op b`; `a` is a register pair or `S0`, `b` one or `S1`.
    fn int_op(&mut self, op: IntOp, dst: VReg, a: Pair, b: Pair) -> Result<(), String> {
        // Add and subtract read `a`'s low word after writing the result's: into the result's
        // register only when it is neither operand's.
        let fresh = match self.target(dst, S2) {
            d if d != a && d != b => d,
            _ => S2,
        };
        match op {
            IntOp::Add => {
                let d = fresh;
                let nc = self.asm.new_label();
                self.asm.word(xt32::add(d.0, a.0, b.0));
                self.asm.word(xt32::add(d.1, a.1, b.1));
                self.asm.branch(BranchCond::Geu, d.0, a.0, nc); // no carry out of the low word
                self.emit(xt32::addi(d.1, d.1, 1))?;
                self.asm.bind(nc);
                self.put(dst, d)
            }
            IntOp::Sub => {
                let d = fresh;
                let nb = self.asm.new_label();
                self.asm.word(xt32::sub(d.0, a.0, b.0));
                self.asm.word(xt32::sub(d.1, a.1, b.1));
                self.asm.branch(BranchCond::Geu, a.0, b.0, nb); // no borrow from the low word
                self.emit(xt32::addi(d.1, d.1, -1))?;
                self.asm.bind(nb);
                self.put(dst, d)
            }
            IntOp::And | IntOp::Or | IntOp::Xor => {
                // Word by word: a result sharing an operand's pair reads each word first.
                let d = self.target(dst, S2);
                let f = match op {
                    IntOp::And => xt32::and,
                    IntOp::Or => xt32::or,
                    _ => xt32::xor,
                };
                self.asm.word(f(d.0, a.0, b.0));
                self.asm.word(f(d.1, a.1, b.1));
                self.put(dst, d)
            }
            IntOp::Mul | IntOp::SDiv | IntOp::SRem | IntOp::UDiv | IntOp::URem => {
                let symbol = match op {
                    IntOp::Mul => "__muldi3",
                    IntOp::SDiv => "__divdi3",
                    IntOp::SRem => "__moddi3",
                    IntOp::UDiv => "__udivdi3",
                    _ => "__umoddi3",
                };
                // `a` is S0 = a10:a11 or a register pair, `b` S1 = a12:a13 or one: moving `a`
                // first cannot clobber `b`.
                self.mov_pair((ARG0, ARG0 + 1), a);
                self.mov_pair((ARG0 + 2, ARG0 + 3), b);
                self.call_symbol(symbol);
                self.put(dst, (ARG0, ARG0 + 1))
            }
            IntOp::Shl | IntOp::Sar => {
                // (long long a, int n): `a` in a10:a11, `n` (the low word of b) in a12.
                self.mov_pair((ARG0, ARG0 + 1), a);
                if b.0 != ARG0 + 2 {
                    self.asm.word(xt32::mov(ARG0 + 2, b.0));
                }
                self.call_symbol(if op == IntOp::Shl {
                    "__ashldi3"
                } else {
                    "__ashrdi3"
                });
                self.put(dst, (ARG0, ARG0 + 1))
            }
        }
    }

    fn fcmp(&mut self, cond: FCond, dst: VReg, a: VReg, b: VReg) -> Result<(), String> {
        // libgcc's comparisons return <0, 0, >0, with unordered operands making each
        // ordered test false (and `__nedf2` non-zero): exactly the IR's semantics.
        let (symbol, bc, x, y) = fcmp_test(cond);
        self.move_into(a, (ARG0, ARG0 + 1))?;
        self.move_into(b, (ARG0 + 2, ARG0 + 3))?;
        self.call_symbol(symbol);
        self.emit(xt32::movi(T2, 0))?;
        self.bool_branch(bc, x, y)?;
        self.put(dst, S2)
    }

    fn inst(&mut self, inst: &Inst) -> Result<(), String> {
        match inst {
            // A constant without a register is rebuilt where it is read.
            Inst::IConst { dst, .. } | Inst::FConst { dst, .. } if self.remat(*dst).is_some() => {}
            Inst::IConst { dst, value } => {
                let d = self.target(*dst, S2);
                self.const64(d, *value)?;
                self.put(*dst, d)?;
            }
            Inst::FConst { dst, value } => {
                let d = self.target(*dst, S2);
                self.const64(d, value.to_bits() as i64)?;
                self.put(*dst, d)?;
            }
            // Integers and doubles share the representation: these are copies.
            Inst::Mov { dst, src }
            | Inst::FloatBits { dst, src }
            | Inst::BitsFloat { dst, src } => {
                // Values sharing a home (coalesced, or a slot reused) need no copy.
                let same_home = self.reg(*src).is_none()
                    && self.reg(*dst).is_none()
                    && self.remat(*src).is_none()
                    && self.remat(*dst).is_none()
                    && self.slot(*src)? == self.slot(*dst)?;
                if same_home {
                    return Ok(());
                }
                if let Some(c) = self.remat(*src) {
                    let d = self.target(*dst, S0);
                    self.const64(d, c)?;
                    return self.put(*dst, d);
                }
                let s = self.operand(*src, S0)?;
                self.put(*dst, s)?;
            }
            // A logic operation on a rebuilt constant uses it as an immediate.
            Inst::Int { op, dst, a, b }
                if matches!(op, IntOp::And | IntOp::Or | IntOp::Xor)
                    && (self.remat(*a).is_some() || self.remat(*b).is_some()) =>
            {
                let (x, c) = match self.remat(*b) {
                    Some(c) => (*a, c),
                    None => (*b, self.remat(*a).unwrap()),
                };
                let px = self.operand(x, S0)?;
                let d = self.target(*dst, S2);
                self.logic_imm(*op, d.0, px.0, c as u32)?;
                self.logic_imm(*op, d.1, px.1, (c >> 32) as u32)?;
                self.put(*dst, d)?;
            }
            Inst::Int { op, dst, a, b } => {
                let pa = self.operand(*a, S0)?;
                let pb = self.operand(*b, S1)?;
                self.int_op(*op, *dst, pa, pb)?;
            }
            Inst::IntImm { op, dst, a, imm }
                if matches!(op, IntOp::Shl | IntOp::Sar) && (0..64).contains(imm) =>
            {
                let pa = self.operand(*a, S0)?;
                let d = self.target(*dst, S2);
                self.shift_imm(*op == IntOp::Shl, *imm as u32, pa, d)?;
                self.put(*dst, d)?;
            }
            Inst::IntImm { op, dst, a, imm }
                if matches!(op, IntOp::And | IntOp::Or | IntOp::Xor) =>
            {
                let pa = self.operand(*a, S0)?;
                let d = self.target(*dst, S2);
                let imm = *imm as i64; // sign-extended, as `const64` would
                self.logic_imm(*op, d.0, pa.0, imm as u32)?;
                self.logic_imm(*op, d.1, pa.1, (imm >> 32) as u32)?;
                self.put(*dst, d)?;
            }
            Inst::IntImm { op, dst, a, imm } => {
                let pa = self.operand(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.int_op(*op, *dst, pa, S1)?;
            }
            Inst::Neg { dst, src } => {
                let s = self.operand(*src, S1)?;
                self.const64(S0, 0)?;
                self.int_op(IntOp::Sub, *dst, S0, s)?;
            }
            Inst::Not { dst, src } => {
                let s = self.operand(*src, S0)?;
                self.emit(xt32::movi(T, -1))?;
                self.asm.word(xt32::xor(S2.0, s.0, T));
                self.asm.word(xt32::xor(S2.1, s.1, T));
                self.put(*dst, S2)?;
            }
            Inst::Popcnt { .. } => {
                return Err("nguvu: ESP32 haina popcnt (jenga bila kuitambua)".into())
            }
            Inst::MulOverflow { .. } => {
                return Err("nguvu: MulOverflow haiungwi mkono kwa ESP32".into())
            }
            Inst::Float { op, dst, a, b } => self.call2(float_symbol(*op), *dst, *a, *b)?,
            Inst::ICmp { cond, dst, a, b } => {
                let pa = self.operand(*a, S0)?;
                let pb = self.operand(*b, S1)?;
                self.icmp(*cond, pa, pb)?;
                self.put(*dst, S2)?;
            }
            Inst::ICmpImm { cond, dst, a, imm } => {
                let pa = self.operand(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.icmp(*cond, pa, S1)?;
                self.put(*dst, S2)?;
            }
            Inst::TestImm { zero, dst, a, imm } => {
                let pa = self.operand(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.asm.word(xt32::and(T, pa.0, S1.0));
                self.asm.word(xt32::and(T2, pa.1, S1.1));
                self.asm.word(xt32::or(T, T, T2));
                self.emit(xt32::movi(T2, 0))?;
                let bc = if *zero {
                    BranchCond::Eq
                } else {
                    BranchCond::Ne
                };
                self.bool_branch(bc, T, T2)?;
                self.put(*dst, S2)?;
            }
            Inst::FCmp { cond, dst, a, b } => self.fcmp(*cond, *dst, *a, *b)?,
            Inst::Select { dst, cond, a, b } => {
                let use_b = self.asm.new_label();
                let done = self.asm.new_label();
                self.nonzero_word(*cond)?;
                self.asm.branch(BranchCond::Eq, T, T2, use_b);
                self.move_into(*a, S2)?;
                self.asm.jump(done);
                self.asm.bind(use_b);
                self.move_into(*b, S2)?;
                self.asm.bind(done);
                self.put(*dst, S2)?;
            }
            Inst::IntToFloat { dst, src } => self.call1("__floatdidf", *dst, *src)?,
            // Only for values proven to be exact integers: truncation is exact.
            Inst::FloatToInt { dst, src } => self.call1("__fixdfdi", *dst, *src)?,
            Inst::Load { dst, base, offset } => {
                let off = u32::try_from(*offset).map_err(|_| "nguvu: anwani hasi")?;
                let b = self.address(*base, S1.0)?;
                // `ld64` reads the base before overwriting it when they share a register.
                let d = self.target(*dst, S0);
                self.ld64(d, b, off)?;
                self.put(*dst, d)?;
            }
            Inst::Store { src, base, offset } => {
                let off = u32::try_from(*offset).map_err(|_| "nguvu: anwani hasi")?;
                let b = self.address(*base, S1.0)?;
                let s = self.operand(*src, S0)?;
                self.st64(s, b, off)?;
            }
            Inst::LoadIndex {
                dst,
                base,
                index,
                kind,
            } => {
                if kind.width() != 8 {
                    // `device::check_lists` admits only lists of doubles on a device.
                    return Err("kosa la ndani: orodha isiyo ya Namba imefika ESP32".into());
                }
                let b = self.address(*base, S1.0)?;
                let i = self.address(*index, S0.0)?;
                self.element_address(b, i);
                let d = self.target(*dst, S2);
                self.ld64(d, T, 0)?;
                self.put(*dst, d)?;
            }
            Inst::StoreIndex {
                src,
                base,
                index,
                kind,
            } => {
                if kind.width() != 8 {
                    return Err("kosa la ndani: orodha isiyo ya Namba imefika ESP32".into());
                }
                let s = self.operand(*src, S2)?;
                let b = self.address(*base, S1.0)?;
                let i = self.address(*index, S0.0)?;
                self.element_address(b, i);
                self.st64(s, T, 0)?;
            }
            // Peripheral registers: one word each, ordered by `memw` as GCC orders volatile
            // accesses on this core.
            Inst::IoLoad { dst, addr } => {
                let a = self.address(*addr, T2)?;
                let d = self.target(*dst, S2);
                self.asm.word(xt32::MEMW);
                self.emit(xt32::l32i(d.0, a, 0))?;
                self.emit(xt32::movi(d.1, 0))?;
                self.put(*dst, d)?;
            }
            Inst::IoStore { src, addr } => {
                let a = self.address(*addr, T2)?;
                let s = self.operand(*src, S0)?;
                self.asm.word(xt32::MEMW);
                self.emit(xt32::s32i(s.0, a, 0))?;
            }
            Inst::CycleCount { dst } => {
                let d = self.target(*dst, S2);
                self.asm.word(xt32::rsr(d.0, xt32::CCOUNT));
                self.emit(xt32::movi(d.1, 0))?;
                self.put(*dst, d)?;
            }
            Inst::StackPointer { dst } => {
                let d = self.target(*dst, S2);
                self.asm.word(xt32::mov(d.0, SP));
                self.emit(xt32::movi(d.1, 0))?;
                self.put(*dst, d)?;
            }
            Inst::CallBuffer { dst } => {
                let off = 8 * self.alloc.slots;
                self.const32(T, off)?;
                let d = self.target(*dst, S2);
                self.asm.word(xt32::add(d.0, SP, T));
                self.emit(xt32::movi(d.1, 0))?;
                self.put(*dst, d)?;
            }
            Inst::CallDirect {
                func,
                args,
                dst,
                result,
            } => {
                if args.len() > 6 {
                    return Err("nguvu: hoja nyingi mno kwa ESP32".into());
                }
                for (i, a) in args.iter().enumerate() {
                    self.low_into(*a, ARG0 + i as u8)?;
                }
                self.pool.call_direct(&mut self.asm, *func);
                self.put(*dst, (ARG0, ARG0 + 1))?;
                self.put(*result, (ARG0 + 2, ARG0 + 3))?;
            }
            Inst::Call {
                target,
                args,
                dst,
                ret32,
                ..
            } => {
                if let Some(h) = helper_index(*target) {
                    // A helper takes its 64-bit argument as two words (a10, a11) and returns
                    // its 64-bit result in a10:a11.
                    self.move_into(args[0], (ARG0, ARG0 + 1))?;
                    self.l32r(T, Lit::Helper(h));
                    self.asm.word(xt32::callx(8, T).expect("callx8"));
                    if let Some(d) = dst {
                        self.put(*d, (ARG0, ARG0 + 1))?;
                    }
                    return Ok(());
                }
                let symbol = runtime_symbol(*target).ok_or_else(|| {
                    format!("inahitaji huduma ya mwenyeji ({target:?}) ambayo kifaa hakina")
                })?;
                // C ABI: a pointer or integer takes the next register, a double the next
                // even-aligned pair.
                let mut r = ARG0;
                for a in args {
                    match self.func.class(*a) {
                        Class::Int => {
                            self.low_into(*a, r)?;
                            r += 1;
                        }
                        Class::Float => {
                            if !r.is_multiple_of(2) {
                                r += 1;
                            }
                            self.move_into(*a, (r, r + 1))?;
                            r += 2;
                        }
                    }
                    if r > 16 {
                        return Err("nguvu: hoja nyingi mno kwa ESP32".into());
                    }
                }
                self.call_symbol(symbol);
                if let Some(d) = dst {
                    if *ret32 && self.func.class(*d) == Class::Int {
                        self.emit(xt32::movi(ARG0 + 1, 0))?;
                    }
                    self.put(*d, (ARG0, ARG0 + 1))?;
                }
            }
        }
        Ok(())
    }

    /// `d = a op c` on one word; a constant of all zeros or all ones decides it outright.
    fn logic_imm(&mut self, op: IntOp, d: u8, a: u8, c: u32) -> Result<(), String> {
        let copy = |g: &mut Self| {
            if d != a {
                g.asm.word(xt32::mov(d, a));
            }
        };
        match (op, c) {
            (IntOp::And, u32::MAX) | (IntOp::Or | IntOp::Xor, 0) => copy(self),
            (IntOp::And, 0) => self.emit(xt32::movi(d, 0))?,
            (IntOp::Or, u32::MAX) => self.emit(xt32::movi(d, -1))?,
            _ => {
                self.const32(T, c)?;
                let f = match op {
                    IntOp::And => xt32::and,
                    IntOp::Or => xt32::or,
                    _ => xt32::xor,
                };
                self.asm.word(f(d, a, T));
            }
        }
        Ok(())
    }

    /// `d = a << k` (`left`) or `a >> k` (arithmetic) for a constant `k` in 0..64, inline:
    /// the funnel shift `src` moves bits across the two words. `d` may be `a` itself: each
    /// form writes a word only after its last read.
    fn shift_imm(&mut self, left: bool, k: u32, a: Pair, d: Pair) -> Result<(), String> {
        match (left, k) {
            (_, 0) => self.mov_pair(d, a),
            (true, 1..=31) => {
                self.emit(xt32::ssai(32 - k))?;
                self.asm.word(xt32::src(d.1, a.1, a.0));
                self.emit(xt32::slli(d.0, a.0, k))?;
            }
            (true, 32) => {
                self.asm.word(xt32::mov(d.1, a.0));
                self.emit(xt32::movi(d.0, 0))?;
            }
            (true, _) => {
                self.emit(xt32::slli(d.1, a.0, k - 32))?;
                self.emit(xt32::movi(d.0, 0))?;
            }
            (false, 1..=31) => {
                self.emit(xt32::ssai(k))?;
                self.asm.word(xt32::src(d.0, a.1, a.0));
                self.emit(xt32::srai(d.1, a.1, k))?;
            }
            (false, 32) => {
                self.asm.word(xt32::mov(d.0, a.1));
                self.emit(xt32::srai(d.1, a.1, 31))?;
            }
            (false, _) => {
                self.emit(xt32::srai(d.0, a.1, k - 32))?;
                self.emit(xt32::srai(d.1, a.1, 31))?;
            }
        }
        Ok(())
    }

    /// `T = base + 8 * index` (low words).
    fn element_address(&mut self, base: u8, index: u8) {
        self.asm.word(xt32::addx8(T, index, base));
    }
}

/// Per block and instruction: a `Load` whose value only the next instruction reads — a double
/// operation taking it as exactly one operand, after which it is dead — for a register every
/// one of whose definitions is such a load. It is loaded straight into that operand's
/// argument registers, so the register needs no home (and a longer-lived value gets its pair).
fn fused_loads(func: &Func) -> Vec<Vec<bool>> {
    let live_out = func.liveness().live_out;
    let candidate = |b: usize, i: usize| -> Option<VReg> {
        let insts = &func.blocks[b].insts;
        let (Inst::Load { dst, .. }, Some(Inst::Float { a, b: y, .. })) =
            (&insts[i], insts.get(i + 1))
        else {
            return None;
        };
        if (a == dst) == (y == dst) {
            return None;
        }
        // Dead after the operation: written again before any read, or never read again.
        for inst in &insts[i + 2..] {
            if inst.uses().contains(dst) {
                return None;
            }
            if inst.defs().contains(dst) {
                return Some(*dst);
            }
        }
        let term_reads = func.blocks[b].term.uses().contains(dst);
        (!term_reads && !live_out[b].contains(*dst)).then_some(*dst)
    };
    let mut fused: Vec<Vec<bool>> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(b, block)| {
            (0..block.insts.len())
                .map(|i| candidate(b, i).is_some())
                .collect()
        })
        .collect();
    // Only registers all of whose definitions are fused loads.
    let mut defs = vec![0u32; func.classes.len()];
    let mut fused_defs = vec![0u32; func.classes.len()];
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, inst) in block.insts.iter().enumerate() {
            for d in inst.defs() {
                defs[d.0 as usize] += 1;
                fused_defs[d.0 as usize] += fused[b][i] as u32;
            }
        }
    }
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, inst) in block.insts.iter().enumerate() {
            if fused[b][i] {
                let d = inst.defs()[0];
                if fused_defs[d.0 as usize] != defs[d.0 as usize] || func.args.contains(&d) {
                    fused[b][i] = false;
                }
            }
        }
    }
    fused
}

/// What an export thunk needs to know about its function (see [`thunk`]).
pub struct ThunkSpec {
    /// Numeric registers and list registers of the function's frame.
    pub num_regs: u32,
    pub list_regs: u32,
    /// Registers that start with a constant: (register, bits of the double).
    pub num_consts: Vec<(u32, u64)>,
    /// Parameters in order: (register, whether it is a list).
    pub params: Vec<(u32, bool)>,
    /// `Return`s of a number or truth value: (pc, register returned).
    pub returns: Vec<(u32, u32)>,
    /// Call the function's direct entry (a number out; list references after the numeric
    /// registers, see `lower::Ctx::direct_lists`): `(host, nums)` in, the number back in
    /// registers with status 0, so no `Return` needs finding.
    pub direct: bool,
    /// The firmware's entry point, `void app_main(void)` (the program's `kuu`): no arguments
    /// read, nothing returned (`params` and `returns` are empty).
    pub entry_point: bool,
}

/// Status kind in the high word of a function's return value meaning "returned"
/// (`native::STATUS_RETURN`).
const STATUS_RETURN: i32 = 3;

/// The export `int32_t asili_f(const hoja *h, double *matokeo)`: builds the function's frame
/// on the stack from `hoja` (a2), calls its ordinary entry (the one entry in `calls`, to be
/// linked), and stores the number returned at `matokeo` (a3). Returns 0, or the non-zero
/// status kind when the call failed.
pub fn thunk(spec: &ThunkSpec) -> Result<Code, String> {
    // Scratch of the thunk's own (it allocates nothing): a4..a7.
    const TS0: Pair = (4, 5);
    const TS1: Pair = (6, 7);
    let nums = 8 * spec.num_regs;
    let lists = 8 * spec.list_regs;
    let host = nums + lists; // depth, runtime table, stack limit: 24 bytes, all zero
    let size = (host + 24).div_ceil(8) * 8;
    let frame = size.div_ceil(16) * 16 + 32;
    if frame > 32760 {
        return Err("kazi ina rejista nyingi mno kwa ESP32".into());
    }
    let (hoja, matokeo) = (2u8, 3u8);
    let mut a = Asm::new();
    let mut pool = Pool::default();
    a.emit(xt32::entry(frame))?;
    // Zero the frame: registers, list references, host fields. A direct entry reads only its
    // parameters and the host fields (its constants and other registers start in code, and
    // its number comes back in registers), so it needs only the host fields zeroed.
    a.emit(xt32::movi(TS1.0, 0))?;
    let zero_from = if spec.direct { host } else { 0 };
    for off in (zero_from..size).step_by(4) {
        store_word(&mut a, &mut pool, TS1.0, off)?;
    }
    // Constants the registers start with (read back by the ordinary entry's `Return`s).
    let consts: &[(u32, u64)] = if spec.direct { &[] } else { &spec.num_consts };
    for &(reg, bits) in consts {
        pool.const32(&mut a, TS0.0, bits as u32)?;
        pool.const32(&mut a, TS0.1, (bits >> 32) as u32)?;
        store_word(&mut a, &mut pool, TS0.0, 8 * reg)?;
        store_word(&mut a, &mut pool, TS0.1, 8 * reg + 4)?;
    }
    // Arguments, eight bytes each in `hoja`.
    for (i, &(reg, is_list)) in spec.params.iter().enumerate() {
        let from = 8 * i as u32;
        if from + 4 > 1020 {
            return Err("ina hoja nyingi mno".into());
        }
        a.emit(xt32::l32i(TS0.0, hoja, from))?;
        a.emit(xt32::l32i(TS0.1, hoja, from + 4))?;
        let to = if is_list { nums + 8 * reg } else { 8 * reg };
        store_word(&mut a, &mut pool, TS0.0, to)?;
        store_word(&mut a, &mut pool, TS0.1, to + 4)?;
    }
    if spec.direct {
        // direct entry(host, nums): status in a10:a11 (0: returned), the number in a12:a13.
        pool.const32(&mut a, T, host)?;
        a.word(xt32::add(ARG0, SP, T));
        a.word(xt32::mov(ARG0 + 1, SP));
        pool.call_direct(&mut a, 0);
        let failed = a.new_label();
        a.word(xt32::or(T, ARG0, ARG0 + 1));
        a.emit(xt32::movi(T2, 0))?;
        a.branch(BranchCond::Ne, T, T2, failed);
        a.emit(xt32::s32i(ARG0 + 2, matokeo, 0))?;
        a.emit(xt32::s32i(ARG0 + 3, matokeo, 4))?;
        a.emit(xt32::movi(2, 0))?;
        a.word(xt32::RETW);
        a.bind(failed);
        a.word(xt32::mov(2, ARG0 + 1)); // the status kind: non-zero for every failure
        a.word(xt32::RETW);
        let mut code = pool.finish(a)?;
        code.frame = frame;
        return Ok(code);
    }
    // entry(runtime: unused on a device, host, frame = list references, nums)
    a.emit(xt32::movi(ARG0, 0))?;
    pool.const32(&mut a, T, host)?;
    a.word(xt32::add(ARG0 + 1, SP, T));
    pool.const32(&mut a, T, nums)?;
    a.word(xt32::add(ARG0 + 2, SP, T));
    a.word(xt32::mov(ARG0 + 3, SP));
    pool.call_direct(&mut a, 0);
    if spec.entry_point {
        a.word(xt32::RETW);
        let mut code = pool.finish(a)?;
        code.frame = frame;
        return Ok(code);
    }
    // a11 = status kind, a10 = the pc of the `Return` reached.
    let fail = a.new_label();
    let done = a.new_label();
    let ok = a.new_label();
    a.emit(xt32::movi(T2, STATUS_RETURN))?;
    a.branch(BranchCond::Eq, ARG0 + 1, T2, ok);
    a.jump(fail);
    a.bind(ok);
    for &(pc, reg) in &spec.returns {
        let next = a.new_label();
        pool.const32(&mut a, T2, pc)?;
        a.branch(BranchCond::Ne, ARG0, T2, next);
        let off = 8 * reg;
        let (b, o) = if off + 4 <= 1020 {
            (SP, off)
        } else {
            pool.const32(&mut a, T, off)?;
            a.word(xt32::add(T, SP, T));
            (T, 0)
        };
        a.emit(xt32::l32i(TS0.0, b, o))?;
        a.emit(xt32::l32i(TS0.1, b, o + 4))?;
        a.emit(xt32::s32i(TS0.0, matokeo, 0))?;
        a.emit(xt32::s32i(TS0.1, matokeo, 4))?;
        a.jump(done);
        a.bind(next);
    }
    a.bind(done);
    a.emit(xt32::movi(2, 0))?;
    a.word(xt32::RETW);
    a.bind(fail);
    a.word(xt32::mov(2, ARG0 + 1));
    a.word(xt32::RETW);
    let mut code = pool.finish(a)?;
    code.frame = frame;
    Ok(code)
}

/// `[sp + off] = r` for any frame offset.
fn store_word(a: &mut Asm, pool: &mut Pool, r: u8, off: u32) -> Result<(), String> {
    if off <= 1020 {
        a.emit(xt32::s32i(r, SP, off))
    } else {
        pool.const32(a, T, off)?;
        a.word(xt32::add(T, SP, T));
        a.emit(xt32::s32i(r, T, 0))
    }
}
