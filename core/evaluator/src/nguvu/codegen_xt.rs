//! Xtensa LX6 (ESP32) code generation from IR, through the [`xt32`](super::xt32) encoder: the
//! device target for strict code (`pata jenga --lengo esp32`, see [`super::device`]).
//!
//! The ESP32 has no double-precision unit and 32-bit registers, so every IR value — a 64-bit
//! integer or a double — lives in an 8-byte home slot as two words (low word first), and each
//! instruction loads its operands into scratch registers, computes, and stores the result.
//! Doubles and the 64-bit operations the core lacks (multiply, divide, shifts, conversions)
//! call the GCC runtime (`__adddf3`, `__muldi3`, ...); these are IEEE-exact, so results equal
//! native code's. No register allocation yet: correctness first.
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

use super::ir::{Class, FCond, FloatOp, Func, ICond, Inst, IntOp, RtFn, Term, VReg};
use super::regalloc::{allocate, Allocation, Target};
use super::xt32::{self, Asm, BranchCond, Label};

/// No register is handed to the allocator: every value has a home slot, shared with values
/// whose live ranges never overlap (this generator keeps nothing in a slot between
/// instructions beyond the value's own life).
pub const TARGET: Target = Target {
    int_callee_saved: &[],
    int_caller_saved: &[],
    float_callee_saved: &[],
    float_caller_saved: &[],
    share_slots: true,
};

type Pair = (u8, u8);

const SP: u8 = 1;
/// Operand scratch pairs (preserved across `call8`, which rotates the window by eight).
const S0: Pair = (4, 5);
const S1: Pair = (6, 7);
/// Result scratch pair; also the return registers.
const S2: Pair = (2, 3);
/// Scratch words (clobbered by calls): addresses, constants, comparison zero.
const T: u8 = 8;
const T2: u8 = 9;
/// First outgoing argument register (`a2` of the callee).
const ARG0: u8 = 10;

/// The runtime functions compiled from IR into the object itself (`device::helper_func`), once
/// each, and called like external functions: their index here names them.
pub const HELPERS: [RtFn; 4] = [
    RtFn::Floor,
    RtFn::Ceil,
    RtFn::FloatToIntSat,
    RtFn::ShiftAmount,
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
        })
    }
}

struct Gen<'f> {
    asm: Asm,
    func: &'f Func,
    alloc: Allocation,
    frame: u32,
    pool: Pool,
}

/// Xtensa code for `func`; an instruction the device cannot run is an error naming it.
pub fn generate(func: &Func) -> Result<Code, String> {
    let alloc = allocate(func, &TARGET);
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
    };
    g.prologue()?;
    let labels: Vec<Label> = (0..func.blocks.len()).map(|_| g.asm.new_label()).collect();
    let order = g.alloc.order.clone();
    for (k, &bi) in order.iter().enumerate() {
        let block = &func.blocks[bi];
        g.asm.bind(labels[bi]);
        let next = order.get(k + 1).map(|&b| labels[b]);
        for inst in &block.insts {
            g.inst(inst)?;
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
                g.load_pair(*v, S2)?;
                g.asm.word(xt32::RETW);
            }
            Term::ReturnNum(v) => {
                g.load_pair(*v, S0)?;
                g.emit(xt32::movi(S2.0, 0))?;
                g.emit(xt32::movi(S2.1, 0))?;
                g.asm.word(xt32::RETW);
            }
        }
    }
    g.pool.finish(g.asm)
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

    fn prologue(&mut self) -> Result<(), String> {
        let frame = self.frame;
        self.emit(xt32::entry(frame))?;
        self.emit(xt32::movi(T2, 0))?;
        // Incoming arguments are pointer-sized, in a2..: zero-extend into their homes.
        for (i, &v) in self.func.args.iter().enumerate() {
            if i >= 6 {
                return Err("nguvu: hoja nyingi mno kwa ESP32".into());
            }
            let off = self.slot(v)?;
            self.st32(2 + i as u8, SP, off)?;
            self.st32(T2, SP, off + 4)?;
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

    fn load_pair(&mut self, v: VReg, p: Pair) -> Result<(), String> {
        let off = self.slot(v)?;
        self.ld64(p, SP, off)
    }

    fn store_pair(&mut self, v: VReg, p: Pair) -> Result<(), String> {
        let off = self.slot(v)?;
        self.st64(p, SP, off)
    }

    fn mov_pair(&mut self, dst: Pair, src: Pair) {
        if dst != src {
            self.asm.word(xt32::mov(dst.0, src.0));
            self.asm.word(xt32::mov(dst.1, src.1));
        }
    }

    // ----- calls --------------------------------------------------------------------------

    /// `callx8` the external function `symbol` (arguments already in `a10`..).
    fn call_symbol(&mut self, symbol: &str) {
        self.l32r(T, Lit::Sym(symbol.to_string()));
        self.asm.word(xt32::callx(8, T).expect("callx8"));
    }

    /// `dst = symbol(a, b)` on two 64-bit values (doubles or integers), result 64-bit.
    fn call2(&mut self, symbol: &str, dst: VReg, a: VReg, b: VReg) -> Result<(), String> {
        self.load_pair(a, (ARG0, ARG0 + 1))?;
        self.load_pair(b, (ARG0 + 2, ARG0 + 3))?;
        self.call_symbol(symbol);
        self.store_pair(dst, (ARG0, ARG0 + 1))
    }

    fn call1(&mut self, symbol: &str, dst: VReg, a: VReg) -> Result<(), String> {
        self.load_pair(a, (ARG0, ARG0 + 1))?;
        self.call_symbol(symbol);
        self.store_pair(dst, (ARG0, ARG0 + 1))
    }

    // ----- comparisons --------------------------------------------------------------------

    /// `T = lo | hi` of `v`, `T2 = 0`: `v != 0` exactly when `T != T2`.
    fn nonzero_word(&mut self, v: VReg) -> Result<(), String> {
        self.load_pair(v, S0)?;
        self.asm.word(xt32::or(T, S0.0, S0.1));
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

    /// `S2 = a cond b` for the pairs in `S0` and `S1`.
    fn icmp(&mut self, cond: ICond) -> Result<(), String> {
        match cond {
            ICond::Lt => self.lt64(S0, S1, true, false),
            ICond::Ge => self.lt64(S0, S1, true, true),
            ICond::Gt => self.lt64(S1, S0, true, false),
            ICond::Le => self.lt64(S1, S0, true, true),
            ICond::Ult => self.lt64(S0, S1, false, false),
            ICond::Ule => self.lt64(S1, S0, false, true),
            ICond::Eq => self.eq64(S0, S1, false),
            ICond::Ne => self.eq64(S0, S1, true),
        }
    }

    // ----- integer arithmetic -------------------------------------------------------------

    /// `dst = a op b` for the pairs in `S0` (a) and `S1` (b).
    fn int_op(&mut self, op: IntOp, dst: VReg) -> Result<(), String> {
        match op {
            IntOp::Add => {
                let nc = self.asm.new_label();
                self.asm.word(xt32::add(S2.0, S0.0, S1.0));
                self.asm.word(xt32::add(S2.1, S0.1, S1.1));
                self.asm.branch(BranchCond::Geu, S2.0, S0.0, nc); // no carry out of the low word
                self.emit(xt32::addi(S2.1, S2.1, 1))?;
                self.asm.bind(nc);
                self.store_pair(dst, S2)
            }
            IntOp::Sub => {
                let nb = self.asm.new_label();
                self.asm.word(xt32::sub(S2.0, S0.0, S1.0));
                self.asm.word(xt32::sub(S2.1, S0.1, S1.1));
                self.asm.branch(BranchCond::Geu, S0.0, S1.0, nb); // no borrow from the low word
                self.emit(xt32::addi(S2.1, S2.1, -1))?;
                self.asm.bind(nb);
                self.store_pair(dst, S2)
            }
            IntOp::And | IntOp::Or | IntOp::Xor => {
                let f = match op {
                    IntOp::And => xt32::and,
                    IntOp::Or => xt32::or,
                    _ => xt32::xor,
                };
                self.asm.word(f(S2.0, S0.0, S1.0));
                self.asm.word(f(S2.1, S0.1, S1.1));
                self.store_pair(dst, S2)
            }
            IntOp::Mul | IntOp::SDiv | IntOp::SRem | IntOp::UDiv | IntOp::URem => {
                let symbol = match op {
                    IntOp::Mul => "__muldi3",
                    IntOp::SDiv => "__divdi3",
                    IntOp::SRem => "__moddi3",
                    IntOp::UDiv => "__udivdi3",
                    _ => "__umoddi3",
                };
                self.mov_pair((ARG0, ARG0 + 1), S0);
                self.mov_pair((ARG0 + 2, ARG0 + 3), S1);
                self.call_symbol(symbol);
                self.store_pair(dst, (ARG0, ARG0 + 1))
            }
            IntOp::Shl | IntOp::Sar => {
                // (long long a, int n): `a` in a10:a11, `n` (the low word of b) in a12.
                self.mov_pair((ARG0, ARG0 + 1), S0);
                self.asm.word(xt32::mov(ARG0 + 2, S1.0));
                self.call_symbol(if op == IntOp::Shl {
                    "__ashldi3"
                } else {
                    "__ashrdi3"
                });
                self.store_pair(dst, (ARG0, ARG0 + 1))
            }
        }
    }

    fn fcmp(&mut self, cond: FCond, dst: VReg, a: VReg, b: VReg) -> Result<(), String> {
        // libgcc's comparisons return <0, 0, >0, with unordered operands making each
        // ordered test false (and `__nedf2` non-zero): exactly the IR's semantics.
        let (symbol, bc, x, y) = match cond {
            FCond::Olt => ("__ltdf2", BranchCond::Lt, ARG0, T2),
            FCond::Ole => ("__ledf2", BranchCond::Ge, T2, ARG0),
            FCond::Ogt => ("__gtdf2", BranchCond::Lt, T2, ARG0),
            FCond::Oge => ("__gedf2", BranchCond::Ge, ARG0, T2),
            FCond::Oeq => ("__eqdf2", BranchCond::Eq, ARG0, T2),
            FCond::Une => ("__nedf2", BranchCond::Ne, ARG0, T2),
        };
        self.load_pair(a, (ARG0, ARG0 + 1))?;
        self.load_pair(b, (ARG0 + 2, ARG0 + 3))?;
        self.call_symbol(symbol);
        self.emit(xt32::movi(T2, 0))?;
        self.bool_branch(bc, x, y)?;
        self.store_pair(dst, S2)
    }

    fn inst(&mut self, inst: &Inst) -> Result<(), String> {
        match inst {
            Inst::IConst { dst, value } => {
                self.const64(S2, *value)?;
                self.store_pair(*dst, S2)?;
            }
            Inst::FConst { dst, value } => {
                self.const64(S2, value.to_bits() as i64)?;
                self.store_pair(*dst, S2)?;
            }
            // Integers and doubles share the slot representation: these are copies.
            Inst::Mov { dst, src }
            | Inst::FloatBits { dst, src }
            | Inst::BitsFloat { dst, src } => {
                self.load_pair(*src, S0)?;
                self.store_pair(*dst, S0)?;
            }
            Inst::Int { op, dst, a, b } => {
                self.load_pair(*a, S0)?;
                self.load_pair(*b, S1)?;
                self.int_op(*op, *dst)?;
            }
            Inst::IntImm { op, dst, a, imm } => {
                self.load_pair(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.int_op(*op, *dst)?;
            }
            Inst::Neg { dst, src } => {
                self.load_pair(*src, S1)?;
                self.const64(S0, 0)?;
                self.int_op(IntOp::Sub, *dst)?;
            }
            Inst::Not { dst, src } => {
                self.load_pair(*src, S0)?;
                self.emit(xt32::movi(T, -1))?;
                self.asm.word(xt32::xor(S2.0, S0.0, T));
                self.asm.word(xt32::xor(S2.1, S0.1, T));
                self.store_pair(*dst, S2)?;
            }
            Inst::Popcnt { .. } => {
                return Err("nguvu: ESP32 haina popcnt (jenga bila kuitambua)".into())
            }
            Inst::MulOverflow { .. } => {
                return Err("nguvu: MulOverflow haiungwi mkono kwa ESP32".into())
            }
            Inst::Float { op, dst, a, b } => {
                let symbol = match op {
                    FloatOp::Add => "__adddf3",
                    FloatOp::Sub => "__subdf3",
                    FloatOp::Mul => "__muldf3",
                    FloatOp::Div => "__divdf3",
                };
                self.call2(symbol, *dst, *a, *b)?;
            }
            Inst::ICmp { cond, dst, a, b } => {
                self.load_pair(*a, S0)?;
                self.load_pair(*b, S1)?;
                self.icmp(*cond)?;
                self.store_pair(*dst, S2)?;
            }
            Inst::ICmpImm { cond, dst, a, imm } => {
                self.load_pair(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.icmp(*cond)?;
                self.store_pair(*dst, S2)?;
            }
            Inst::TestImm { zero, dst, a, imm } => {
                self.load_pair(*a, S0)?;
                self.const64(S1, *imm as i64)?;
                self.asm.word(xt32::and(T, S0.0, S1.0));
                self.asm.word(xt32::and(T2, S0.1, S1.1));
                self.asm.word(xt32::or(T, T, T2));
                self.emit(xt32::movi(T2, 0))?;
                let bc = if *zero {
                    BranchCond::Eq
                } else {
                    BranchCond::Ne
                };
                self.bool_branch(bc, T, T2)?;
                self.store_pair(*dst, S2)?;
            }
            Inst::FCmp { cond, dst, a, b } => self.fcmp(*cond, *dst, *a, *b)?,
            Inst::Select { dst, cond, a, b } => {
                let use_b = self.asm.new_label();
                let done = self.asm.new_label();
                self.nonzero_word(*cond)?;
                self.asm.branch(BranchCond::Eq, T, T2, use_b);
                self.load_pair(*a, S2)?;
                self.asm.jump(done);
                self.asm.bind(use_b);
                self.load_pair(*b, S2)?;
                self.asm.bind(done);
                self.store_pair(*dst, S2)?;
            }
            Inst::IntToFloat { dst, src } => self.call1("__floatdidf", *dst, *src)?,
            // Only for values proven to be exact integers: truncation is exact.
            Inst::FloatToInt { dst, src } => self.call1("__fixdfdi", *dst, *src)?,
            Inst::Load { dst, base, offset } => {
                let off = u32::try_from(*offset).map_err(|_| "nguvu: anwani hasi")?;
                self.load_pair(*base, S1)?;
                self.ld64(S0, S1.0, off)?;
                self.store_pair(*dst, S0)?;
            }
            Inst::Store { src, base, offset } => {
                let off = u32::try_from(*offset).map_err(|_| "nguvu: anwani hasi")?;
                self.load_pair(*base, S1)?;
                self.load_pair(*src, S0)?;
                self.st64(S0, S1.0, off)?;
            }
            Inst::LoadIndex {
                dst,
                base,
                index,
                kind,
            } => {
                if kind.width() != 8 {
                    return Err(
                        "nguvu: orodha za namba kamili zenye baiti chache haziungwi mkono \
                                kwa ESP32 bado"
                            .into(),
                    );
                }
                self.load_pair(*base, S1)?;
                self.load_pair(*index, S0)?;
                self.element_address()?;
                self.ld64(S2, T, 0)?;
                self.store_pair(*dst, S2)?;
            }
            Inst::StoreIndex {
                src,
                base,
                index,
                kind,
            } => {
                if kind.width() != 8 {
                    return Err(
                        "nguvu: orodha za namba kamili zenye baiti chache haziungwi mkono \
                                kwa ESP32 bado"
                            .into(),
                    );
                }
                self.load_pair(*src, S2)?;
                self.load_pair(*base, S1)?;
                self.load_pair(*index, S0)?;
                self.element_address()?;
                self.st64(S2, T, 0)?;
            }
            Inst::StackPointer { dst } => {
                self.asm.word(xt32::mov(S2.0, SP));
                self.emit(xt32::movi(S2.1, 0))?;
                self.store_pair(*dst, S2)?;
            }
            Inst::CallBuffer { dst } => {
                let off = 8 * self.alloc.slots;
                self.const32(T, off)?;
                self.asm.word(xt32::add(S2.0, SP, T));
                self.emit(xt32::movi(S2.1, 0))?;
                self.store_pair(*dst, S2)?;
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
                    let off = self.slot(*a)?;
                    let (b, o) = self.at(SP, off)?;
                    self.emit(xt32::l32i(ARG0 + i as u8, b, o))?;
                }
                self.pool.call_direct(&mut self.asm, *func);
                self.store_pair(*dst, (ARG0, ARG0 + 1))?;
                self.store_pair(*result, (ARG0 + 2, ARG0 + 3))?;
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
                    self.load_pair(args[0], (ARG0, ARG0 + 1))?;
                    self.l32r(T, Lit::Helper(h));
                    self.asm.word(xt32::callx(8, T).expect("callx8"));
                    if let Some(d) = dst {
                        self.store_pair(*d, (ARG0, ARG0 + 1))?;
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
                            let off = self.slot(*a)?;
                            let (b, o) = self.at(SP, off)?;
                            self.emit(xt32::l32i(r, b, o))?;
                            r += 1;
                        }
                        Class::Float => {
                            if r % 2 != 0 {
                                r += 1;
                            }
                            self.load_pair(*a, (r, r + 1))?;
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
                    self.store_pair(*d, (ARG0, ARG0 + 1))?;
                }
            }
        }
        Ok(())
    }

    /// `T = base + 8 * index` for `base` in `S1` and `index` in `S0` (low words).
    fn element_address(&mut self) -> Result<(), String> {
        self.asm.word(xt32::add(T, S0.0, S0.0)); // 2i
        self.asm.word(xt32::add(T, T, T)); // 4i
        self.asm.word(xt32::add(T, T, T)); // 8i
        self.asm.word(xt32::add(T, T, S1.0));
        Ok(())
    }
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
}

/// Status kind in the high word of a function's return value meaning "returned"
/// (`native::STATUS_RETURN`).
const STATUS_RETURN: i32 = 3;

/// The export `int32_t asili_f(const hoja *h, double *matokeo)`: builds the function's frame
/// on the stack from `hoja` (a2), calls its ordinary entry (the one entry in `calls`, to be
/// linked), and stores the number returned at `matokeo` (a3). Returns 0, or the non-zero
/// status kind when the call failed.
pub fn thunk(spec: &ThunkSpec) -> Result<Code, String> {
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
    // Zero the whole frame: registers, list references, host fields.
    a.emit(xt32::movi(S1.0, 0))?;
    for off in (0..size).step_by(4) {
        store_word(&mut a, &mut pool, S1.0, off)?;
    }
    // Constants the registers start with.
    for &(reg, bits) in &spec.num_consts {
        pool.const32(&mut a, S0.0, bits as u32)?;
        pool.const32(&mut a, S0.1, (bits >> 32) as u32)?;
        store_word(&mut a, &mut pool, S0.0, 8 * reg)?;
        store_word(&mut a, &mut pool, S0.1, 8 * reg + 4)?;
    }
    // Arguments, eight bytes each in `hoja`.
    for (i, &(reg, is_list)) in spec.params.iter().enumerate() {
        let from = 8 * i as u32;
        if from + 4 > 1020 {
            return Err("ina hoja nyingi mno".into());
        }
        a.emit(xt32::l32i(S0.0, hoja, from))?;
        a.emit(xt32::l32i(S0.1, hoja, from + 4))?;
        let to = if is_list { nums + 8 * reg } else { 8 * reg };
        store_word(&mut a, &mut pool, S0.0, to)?;
        store_word(&mut a, &mut pool, S0.1, to + 4)?;
    }
    // entry(runtime: unused on a device, host, frame = list references, nums)
    a.emit(xt32::movi(ARG0, 0))?;
    pool.const32(&mut a, T, host)?;
    a.word(xt32::add(ARG0 + 1, SP, T));
    pool.const32(&mut a, T, nums)?;
    a.word(xt32::add(ARG0 + 2, SP, T));
    a.word(xt32::mov(ARG0 + 3, SP));
    pool.call_direct(&mut a, 0);
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
        a.emit(xt32::l32i(S0.0, b, o))?;
        a.emit(xt32::l32i(S0.1, b, o + 4))?;
        a.emit(xt32::s32i(S0.0, matokeo, 0))?;
        a.emit(xt32::s32i(S0.1, matokeo, 4))?;
        a.jump(done);
        a.bind(next);
    }
    a.bind(done);
    a.emit(xt32::movi(2, 0))?;
    a.word(xt32::RETW);
    a.bind(fail);
    a.word(xt32::mov(2, ARG0 + 1));
    a.word(xt32::RETW);
    pool.finish(a)
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
