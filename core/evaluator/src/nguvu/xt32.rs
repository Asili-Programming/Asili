//! Xtensa LX6 (ESP32) instruction encoder, wide (24-bit) forms only, for the strict-code
//! device target: the instruction words the code generator emits, and label fixups.
//!
//! Every encoding here is checked against `xtensa-esp32-elf-as` in the tests (golden
//! words taken from its output). [`Asm::word`] emits the 16-bit code-density form of an
//! instruction that has one ([`narrow`]); label fixups and placeholders stay 24-bit.
//! Words are little-endian in memory: an instruction is `word & 0xffffff`, stored as three
//! bytes low byte first.
//!
//! Field layout of the 24-bit word:
//! * RRR:  `op2 << 20 | op1 << 16 | r << 12 | s << 8 | t << 4 | op0`
//! * RRI8: `imm8 << 16 | r << 12 | s << 8 | t << 4 | op0`
//! * RI16 (movi, l32r): `imm16 << 8 | t << 4 | op0` (movi splits imm12 into bits 8-11 and 16-23)
//! * CALL/J: `offset18 << 6 | n << 4 | op0`

use std::collections::HashMap;

fn rrr(op2: u32, op1: u32, r: u8, s: u8, t: u8) -> u32 {
    (op2 << 20) | (op1 << 16) | ((r as u32) << 12) | ((s as u32) << 8) | ((t as u32) << 4)
}

fn rri8(op0: u32, r: u32, s: u8, t: u8, imm8: u8) -> u32 {
    ((imm8 as u32) << 16) | (r << 12) | ((s as u32) << 8) | ((t as u32) << 4) | op0
}

/// `add ar, as, at`
pub fn add(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x8, 0, ar, as_, at)
}
/// `sub ar, as, at`
pub fn sub(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0xC, 0, ar, as_, at)
}
/// `and ar, as, at`
pub fn and(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x1, 0, ar, as_, at)
}
/// `or ar, as, at`
pub fn or(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x2, 0, ar, as_, at)
}
/// `xor ar, as, at`
pub fn xor(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x3, 0, ar, as_, at)
}
/// `mull ar, as, at` (low 32 bits of the product)
pub fn mull(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x8, 2, ar, as_, at)
}
/// `mov ar, as` (the assembler's alias for `or ar, as, as`)
pub fn mov(ar: u8, as_: u8) -> u32 {
    or(ar, as_, as_)
}
/// `ssl as`: set the shift amount for `sll` to `32 - as`
pub fn ssl(as_: u8) -> u32 {
    rrr(0x4, 0, 1, as_, 0)
}
/// `ssr as`: set the shift amount for `srl`/`sra` to `as`
pub fn ssr(as_: u8) -> u32 {
    rrr(0x4, 0, 0, as_, 0)
}
/// `sll ar, as`
pub fn sll(ar: u8, as_: u8) -> u32 {
    rrr(0xA, 1, ar, as_, 0)
}
/// `srl ar, at`
pub fn srl(ar: u8, at: u8) -> u32 {
    rrr(0x9, 1, ar, 0, at)
}
/// `sra ar, at`
pub fn sra(ar: u8, at: u8) -> u32 {
    rrr(0xB, 1, ar, 0, at)
}
/// `slli ar, as, sa` (sa in 1..=31; the field holds `32 - sa`)
pub fn slli(ar: u8, as_: u8, sa: u32) -> Option<u32> {
    if !(1..=31).contains(&sa) {
        return None;
    }
    let f = 32 - sa;
    Some(rrr(f >> 4, 1, ar, as_, (f & 0xf) as u8))
}
/// `srai ar, at, sa` (sa in 0..=31)
pub fn srai(ar: u8, at: u8, sa: u32) -> Option<u32> {
    if sa > 31 {
        return None;
    }
    Some(rrr(2 | (sa >> 4), 1, ar, (sa & 0xf) as u8, at))
}
/// `ssai sa`: set the shift amount for `src`/`srl`/`sra` to `sa` (0..=31)
pub fn ssai(sa: u32) -> Option<u32> {
    if sa > 31 {
        return None;
    }
    Some(rrr(4, 0, 4, (sa & 0xf) as u8, (sa >> 4) as u8))
}
/// `src ar, as, at`: `ar` = the low word of `(as:at) >> SAR` (a funnel shift)
pub fn src(ar: u8, as_: u8, at: u8) -> u32 {
    rrr(0x8, 1, ar, as_, at)
}
/// `addi at, as, imm8` (imm8 in -128..=127)
pub fn addi(at: u8, as_: u8, imm: i32) -> Option<u32> {
    if !(-128..=127).contains(&imm) {
        return None;
    }
    Some(rri8(0x2, 0xC, as_, at, imm as i8 as u8))
}
/// `l32i at, as, off` (off a multiple of 4 in 0..=1020)
pub fn l32i(at: u8, as_: u8, off: u32) -> Option<u32> {
    if off % 4 != 0 || off > 1020 {
        return None;
    }
    Some(rri8(0x2, 0x2, as_, at, (off / 4) as u8))
}
/// `s32i at, as, off` (off a multiple of 4 in 0..=1020)
pub fn s32i(at: u8, as_: u8, off: u32) -> Option<u32> {
    if off % 4 != 0 || off > 1020 {
        return None;
    }
    Some(rri8(0x2, 0x6, as_, at, (off / 4) as u8))
}
/// `movi at, imm` (imm12 signed: -2048..=2047)
pub fn movi(at: u8, imm: i32) -> Option<u32> {
    if !(-2048..=2047).contains(&imm) {
        return None;
    }
    let v = (imm as u32) & 0xfff;
    Some(((v & 0xff) << 16) | (((v >> 8) & 0xf) << 8) | (0xA << 12) | ((at as u32) << 4) | 0x2)
}
/// `entry a1, frame` (frame: a multiple of 8 in 0..=32760)
pub fn entry(frame: u32) -> Option<u32> {
    if frame % 8 != 0 || frame > 32760 {
        return None;
    }
    Some(((frame / 8) << 12) | (1 << 8) | (3 << 4) | 0x6)
}
/// `retw` (windowed return)
pub const RETW: u32 = 0x000090;
/// `ret` (call0 return)
pub const RET: u32 = 0x000080;
/// `callx{0,4,8,12} as`: `n` is 0, 4, 8 or 12
pub fn callx(n: u8, as_: u8) -> Option<u32> {
    if n % 4 != 0 || n > 12 {
        return None;
    }
    Some(((0xC + (n as u32) / 4) << 4) | ((as_ as u32) << 8))
}

/// `l32r at` reading the literal `words` 32-bit words before the aligned pc (`words < 0`).
pub fn l32r_word(at: u8, words: i64) -> Option<u32> {
    if !(-32768..=-1).contains(&words) {
        return None;
    }
    Some((((words as u32) & 0xffff) << 8) | ((at as u32) << 4) | 0x1)
}

/// The word offset an `l32r` at byte `pc` needs to read the literal at byte `literal`.
pub fn l32r_offset(pc: usize, literal: usize) -> i64 {
    (literal as i64 - ((pc as i64 + 3) & !3)) / 4
}

/// The 16-bit code-density form of 24-bit instruction `w`, where one exists: `add.n`,
/// `addi.n` (immediates -1 and 1..=15), `mov.n`, `movi.n` (-32..=95), `l32i.n`/`s32i.n`
/// (offsets 0..=60) and `ret.n`/`retw.n`.
pub fn narrow(w: u32) -> Option<u16> {
    let op0 = w & 0xf;
    let t = (w >> 4) & 0xf;
    let s = (w >> 8) & 0xf;
    let r = (w >> 12) & 0xf;
    let op1 = (w >> 16) & 0xf;
    let op2 = (w >> 20) & 0xf;
    let imm8 = (w >> 16) & 0xff;
    let n = |a: u32, b: u32, c: u32, op: u32| Some(((a << 12) | (b << 8) | (c << 4) | op) as u16);
    match op0 {
        0 if w == RETW => Some(0xf01d),
        0 if w == RET => Some(0xf00d),
        0 if op1 == 0 && op2 == 0x8 => n(r, s, t, 0xA), // add ar, as, at
        0 if op1 == 0 && op2 == 0x2 && s == t => n(0, s, r, 0xD), // mov ar, as
        2 if r == 0x2 && imm8 <= 15 => n(imm8, s, t, 0x8), // l32i at, as, 4*imm8
        2 if r == 0x6 && imm8 <= 15 => n(imm8, s, t, 0x9), // s32i at, as, 4*imm8
        2 if r == 0xC => {
            // addi at, as, imm8
            let imm = imm8 as u8 as i8 as i32;
            match imm {
                -1 => n(t, s, 0, 0xB),
                1..=15 => n(t, s, imm as u32, 0xB),
                _ => None,
            }
        }
        2 if r == 0xA => {
            // movi at, imm12
            let raw = (((w >> 8) & 0xf) << 8) | imm8;
            let imm = ((raw << 20) as i32) >> 20;
            if (-32..=95).contains(&imm) {
                let v = (imm as u32) & 0x7f;
                n(v & 0xf, t, (v >> 4) & 0x7, 0xC)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Branch conditions of the RRI8 branch group (`op0 = 7`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchCond {
    Eq,
    Ne,
    Lt,
    Ge,
    Ltu,
    Geu,
}

impl BranchCond {
    fn r(self) -> u32 {
        match self {
            BranchCond::Eq => 0x1,
            BranchCond::Lt => 0x2,
            BranchCond::Ltu => 0x3,
            BranchCond::Ne => 0x9,
            BranchCond::Ge => 0xA,
            BranchCond::Geu => 0xB,
        }
    }
}

/// A label in the code being assembled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label(pub u32);

#[derive(Clone, Copy, Debug)]
enum Fix {
    /// 8-bit pc-relative branch at byte `at`, `(s, t, cond)`.
    Branch {
        at: usize,
        s: u8,
        t: u8,
        cond: BranchCond,
    },
    /// 18-bit pc-relative `j` at byte `at`.
    Jump { at: usize },
    /// 18-bit pc-relative `call{0,4,8,12}` at byte `at`, with the `n` field.
    Call { at: usize, n: u8 },
    /// `l32r at` at byte `at`: literal at the label (word aligned, before the instruction).
    Literal { at: usize, t: u8 },
}

/// An Xtensa code buffer: instruction words plus pending label references.
#[derive(Default)]
pub struct Asm {
    code: Vec<u8>,
    labels: HashMap<u32, usize>,
    fixups: Vec<(Fix, Label)>,
    next_label: u32,
}

impl Asm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Byte offset of the next instruction.
    pub fn pc(&self) -> usize {
        self.code.len()
    }

    pub fn new_label(&mut self) -> Label {
        self.next_label += 1;
        Label(self.next_label)
    }

    pub fn bind(&mut self, l: Label) {
        self.labels.insert(l.0, self.code.len());
    }

    /// Emits an instruction: its 16-bit code-density form when it has one, else the 24-bit
    /// word.
    pub fn word(&mut self, w: u32) {
        let w = w & 0xff_ffff;
        if let Some(h) = narrow(w) {
            self.code.extend_from_slice(&h.to_le_bytes());
        } else {
            self.wide(w);
        }
    }

    /// Emits the 24-bit word `w` as it is (placeholders patched later must keep their size).
    fn wide(&mut self, w: u32) {
        self.code
            .extend_from_slice(&(w & 0xff_ffff).to_le_bytes()[..3]);
    }

    /// A 3-byte placeholder for an instruction patched later.
    pub fn placeholder(&mut self) {
        self.wide(0);
    }

    /// Emits a 32-bit literal (data, not an instruction), aligned to 4 bytes.
    pub fn literal(&mut self, v: u32) {
        self.align4();
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    /// Pads with zero bytes to the next 4-byte boundary.
    pub fn align4(&mut self) {
        while self.code.len() % 4 != 0 {
            self.code.push(0);
        }
    }

    /// Emits an instruction that must exist (the operands are in range).
    pub fn emit(&mut self, w: Option<u32>) -> Result<(), String> {
        match w {
            Some(w) => {
                self.word(w);
                Ok(())
            }
            None => Err("thamani iko nje ya mpaka wa agizo la Xtensa".into()),
        }
    }

    pub fn branch(&mut self, cond: BranchCond, s: u8, t: u8, to: Label) {
        let at = self.pc();
        self.fixups.push((Fix::Branch { at, s, t, cond }, to));
        self.placeholder();
    }

    pub fn jump(&mut self, to: Label) {
        let at = self.pc();
        self.fixups.push((Fix::Jump { at }, to));
        self.placeholder();
    }

    /// `call{n}` to a label in the same buffer (n = 0, 4, 8 or 12).
    pub fn call(&mut self, n: u8, to: Label) -> Result<(), String> {
        if n % 4 != 0 || n > 12 {
            return Err("n ya call lazima iwe 0, 4, 8 au 12".into());
        }
        let at = self.pc();
        self.fixups.push((Fix::Call { at, n: n / 4 }, to));
        self.placeholder();
        Ok(())
    }

    /// `l32r at, literal` where the literal word is placed at `to` (before this instruction).
    pub fn l32r(&mut self, t: u8, to: Label) {
        let at = self.pc();
        self.fixups.push((Fix::Literal { at, t }, to));
        self.placeholder();
    }

    /// Patches every pending reference and returns the code bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let fixups = std::mem::take(&mut self.fixups);
        for (fix, to) in fixups {
            let target = *self
                .labels
                .get(&to.0)
                .ok_or_else(|| "lebo haijafungwa".to_string())? as i64;
            match fix {
                Fix::Branch { at, s, t, cond } => {
                    let off = target - (at as i64 + 4);
                    if !(-128..=127).contains(&off) {
                        return Err("tawi liko nje ya mpaka wa baiti 127".into());
                    }
                    let w = rri8(0x7, cond.r(), s, t, off as i8 as u8);
                    self.patch(at, w);
                }
                Fix::Jump { at } => {
                    let off = target - (at as i64 + 4);
                    self.patch(at, ((off as u32 & 0x3_ffff) << 6) | 0x6);
                }
                Fix::Call { at, n } => {
                    let base = (at as i64 + 4) & !3;
                    if target % 4 != 0 {
                        return Err("lengo la call lazima liwe la mpangilio wa baiti 4".into());
                    }
                    let off = (target - base) / 4;
                    self.patch(at, ((off as u32 & 0x3_ffff) << 6) | ((n as u32) << 4) | 0x5);
                }
                Fix::Literal { at, t } => {
                    let base = (at as i64 + 3) & !3;
                    if target % 4 != 0 {
                        return Err("neno la literal lazima liwe la mpangilio wa baiti 4".into());
                    }
                    let off = (target - base) / 4;
                    if !(-32768..=-1).contains(&off) {
                        return Err("neno la literal liko mbele ya maagizo".into());
                    }
                    self.patch(at, (((off as u32) & 0xffff) << 8) | ((t as u32) << 4) | 0x1);
                }
            }
        }
        Ok(self.code)
    }

    fn patch(&mut self, at: usize, w: u32) {
        let b = (w & 0xff_ffff).to_le_bytes();
        self.code[at..at + 3].copy_from_slice(&b[..3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words checked against `xtensa-esp32-elf-as` (wide forms; see the module doc).
    #[test]
    fn register_forms_match_the_assembler() {
        assert_eq!(add(2, 3, 4), 0x802340);
        assert_eq!(sub(2, 3, 4), 0xc02340);
        assert_eq!(and(2, 3, 4), 0x102340);
        assert_eq!(or(2, 3, 4), 0x202340);
        assert_eq!(xor(2, 3, 4), 0x302340);
        assert_eq!(mull(2, 3, 4), 0x822340);
        assert_eq!(sll(2, 3), 0xa12300);
        assert_eq!(srl(2, 3), 0x912030);
        assert_eq!(sra(2, 3), 0xb12030);
        assert_eq!(ssl(3), 0x401300);
        assert_eq!(ssr(3), 0x400300);
        assert_eq!(RET, 0x000080);
        assert_eq!(RETW, 0x000090);
        assert_eq!(callx(0, 2), Some(0x0002c0));
        assert_eq!(callx(8, 2), Some(0x0002e0));
        assert_eq!(callx(12, 2), Some(0x0002f0));
    }

    /// 16-bit forms, checked against `xtensa-esp32-elf-as` (its default, density) and GCC.
    #[test]
    fn narrow_forms_match_the_assembler() {
        assert_eq!(narrow(add(2, 3, 4)), Some(0x234a));
        assert_eq!(narrow(addi(2, 3, 5).unwrap()), Some(0x235b));
        assert_eq!(narrow(addi(2, 3, -1).unwrap()), Some(0x230b));
        assert_eq!(narrow(addi(2, 3, 16).unwrap()), None);
        assert_eq!(narrow(l32i(2, 3, 8).unwrap()), Some(0x2328));
        assert_eq!(narrow(s32i(2, 3, 8).unwrap()), Some(0x2329));
        assert_eq!(narrow(l32i(2, 3, 64).unwrap()), None);
        assert_eq!(narrow(mov(10, 2)), Some(0x02ad));
        assert_eq!(narrow(movi(12, 3).unwrap()), Some(0x3c0c));
        assert_eq!(narrow(movi(12, 100).unwrap()), None);
        assert_eq!(narrow(RETW), Some(0xf01d));
        assert_eq!(narrow(RET), Some(0xf00d));
        assert_eq!(narrow(movi(2, -32).unwrap()), Some(0x026c));
        assert_eq!(narrow(movi(2, -1).unwrap()), Some(0xf27c));
        assert_eq!(narrow(movi(2, 95).unwrap()), Some(0xf25c));
        assert_eq!(narrow(movi(7, 0).unwrap()), Some(0x070c));
        assert_eq!(narrow(mov(3, 4)), Some(0x043d));
        assert_eq!(narrow(addi(5, 6, 15).unwrap()), Some(0x56fb));
        assert_eq!(narrow(l32i(4, 1, 60).unwrap()), Some(0xf148));
        assert_eq!(narrow(s32i(4, 1, 0).unwrap()), Some(0x0149));
        assert_eq!(narrow(sub(2, 3, 4)), None);
        assert_eq!(narrow(or(2, 3, 4)), None);
    }

    /// Shift-immediate forms, checked against `xtensa-esp32-elf-as`.
    #[test]
    fn shift_forms_match_the_assembler() {
        assert_eq!(slli(2, 3, 1), Some(0x1123f0));
        assert_eq!(slli(2, 3, 5), Some(0x1123b0));
        assert_eq!(slli(2, 3, 16), Some(0x112300));
        assert_eq!(slli(2, 3, 31), Some(0x012310));
        assert_eq!(slli(2, 3, 0), None);
        assert_eq!(srai(2, 3, 0), Some(0x212030));
        assert_eq!(srai(2, 3, 1), Some(0x212130));
        assert_eq!(srai(2, 3, 16), Some(0x312030));
        assert_eq!(srai(2, 3, 31), Some(0x312f30));
        assert_eq!(ssai(0), Some(0x404000));
        assert_eq!(ssai(1), Some(0x404100));
        assert_eq!(ssai(16), Some(0x404010));
        assert_eq!(ssai(31), Some(0x404f10));
        assert_eq!(src(2, 3, 4), 0x812340);
    }

    #[test]
    fn immediate_forms_match_the_assembler() {
        assert_eq!(addi(2, 3, 5), Some(0x05c322));
        assert_eq!(addi(2, 3, -1), Some(0xffc322));
        assert_eq!(addi(2, 3, 128), None);
        assert_eq!(l32i(2, 3, 8), Some(0x022322));
        assert_eq!(s32i(2, 3, 8), Some(0x026322));
        assert_eq!(l32i(2, 3, 2), None);
        assert_eq!(movi(2, 100), Some(0x64a022));
        assert_eq!(movi(2, -2048), Some(0x00a822));
        assert_eq!(movi(2, 2047), Some(0xffa722));
        assert_eq!(movi(2, 2048), None);
        assert_eq!(entry(32), Some(0x004136));
        assert_eq!(entry(4088), Some(0x1ff136));
        assert_eq!(entry(12), None);
    }

    /// Branch and jump encodings, with the offsets the assembler produced for the same code.
    #[test]
    fn branches_and_jumps_match_the_assembler() {
        // beq a2, a3 with its target 14 bytes past the end of the branch: the assembler's 0x0e1237.
        let mut a = Asm::new();
        let l = a.new_label();
        a.branch(BranchCond::Eq, 2, 3, l);
        for _ in 0..5 {
            a.word(sub(2, 3, 4)); // 15 bytes, so the label is at byte 18 = 4 + 14
        }
        a.bind(l);
        let code = a.finish().unwrap();
        assert_eq!(&code[0..3], &[0x37, 0x12, 0x0e]);

        // j to the very next instruction: offset -1, the assembler's 0xffffc6.
        let mut a = Asm::new();
        let l = a.new_label();
        a.jump(l);
        a.bind(l);
        a.word(RET);
        let code = a.finish().unwrap();
        assert_eq!(&code[0..3], &[0xc6, 0xff, 0xff]);
    }

    #[test]
    fn literal_load_fixup() {
        // literal word at bytes 0..4; l32r at byte 4 reads it: the base is (4+3)&!3 = 4 and
        // the target 0, so the word offset is -1, giving 0xffff21 (bytes 21 ff ff).
        let mut a = Asm::new();
        let lit = a.new_label();
        a.bind(lit);
        a.literal(0x1234_5678);
        a.l32r(2, lit);
        let code = a.finish().unwrap();
        assert_eq!(&code[4..7], &[0x21, 0xff, 0xff]);
    }

    #[test]
    fn call_fixup() {
        // call8 at byte 0 to a word-aligned label at byte 16: base (0+4)&!3 = 4, so the word
        // offset is (16-4)/4 = 3 and the word is 3<<6 | 2<<4 | 5 = 0xe5.
        let mut a = Asm::new();
        let l = a.new_label();
        a.call(8, l).unwrap();
        for _ in 0..4 {
            a.word(sub(2, 3, 4)); // bytes 3..15
        }
        a.align4(); // byte 16
        a.bind(l);
        let code = a.finish().unwrap();
        assert_eq!(&code[0..3], &[0xe5, 0x00, 0x00]);
    }

    #[test]
    fn unaligned_call_target_is_rejected() {
        let mut a = Asm::new();
        let l = a.new_label();
        a.call(0, l).unwrap();
        a.word(sub(2, 3, 4)); // label at byte 6: not word-aligned
        a.bind(l);
        assert!(a.finish().is_err());
    }
}
