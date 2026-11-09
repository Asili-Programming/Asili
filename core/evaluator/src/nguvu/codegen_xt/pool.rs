//! A function's code and its literal pool: the words `l32r` loads (constants, and the addresses
//! calls go through, relocated when the object is laid out), placed right before the code.

use super::*;

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
pub(super) enum Lit {
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
pub(super) struct Pool {
    lits: Vec<Lit>,
    /// (code offset of the `l32r`, literal index, register)
    uses: Vec<(usize, usize, u8)>,
}

impl Pool {
    pub(super) fn literal(&mut self, lit: Lit) -> usize {
        if let Some(i) = self.lits.iter().position(|l| *l == lit) {
            return i;
        }
        self.lits.push(lit);
        self.lits.len() - 1
    }

    pub(super) fn l32r(&mut self, asm: &mut Asm, reg: u8, lit: Lit) {
        let index = self.literal(lit);
        self.uses.push((asm.pc(), index, reg));
        asm.word(0);
    }

    /// `reg = v`
    pub(super) fn const32(&mut self, asm: &mut Asm, reg: u8, v: u32) -> Result<(), String> {
        let s = v as i32;
        if (-2048..=2047).contains(&s) {
            asm.emit(xt32::movi(reg, s))
        } else {
            self.l32r(asm, reg, Lit::Word(v));
            Ok(())
        }
    }

    /// Call program function `callee` (its address is relocated when the image is laid out).
    pub(super) fn call_direct(&mut self, asm: &mut Asm, callee: u32) {
        self.l32r(asm, T, Lit::Func(callee));
        asm.word(xt32::callx(8, T).expect("callx8"));
    }

    /// Lays out the literal pool before the code and patches every `l32r`.
    pub(super) fn finish(self, asm: Asm) -> Result<Code, String> {
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
