//! The ELF32 relocatable object both device targets write (ARM EABI5 or Xtensa).

use super::*;

/// `R_XTENSA_32`: a 32-bit absolute address (the literals naming external functions).
pub(super) const R_XTENSA_32: u32 = 1;

/// A relocation in `.text`: at byte `at`, of type `kind`, against undefined symbol `symbol` —
/// or against `.text` itself with `addend` when `symbol` is `None` (Xtensa only: ARM's REL
/// relocations carry no addend).
pub(super) struct ElfReloc {
    pub(super) at: usize,
    pub(super) symbol: Option<String>,
    pub(super) kind: u32,
    pub(super) addend: u32,
}

/// An ELF32 relocatable object holding `text`, with a global function symbol for each export
/// and its relocations: for ARM (EABI5, hard float) Thumb symbols and REL relocations, for
/// Xtensa RELA relocations.
pub(super) fn elf_object(
    lengo: Lengo,
    text: &[u8],
    exports: &[(String, usize, usize)],
    relocs: &[ElfReloc],
) -> Vec<u8> {
    let arm = lengo == Lengo::CortexM;
    fn u16le(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn u32le(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    let mut strtab = vec![0u8];
    let name = |s: &str, tab: &mut Vec<u8>| -> u32 {
        let at = tab.len() as u32;
        tab.extend_from_slice(s.as_bytes());
        tab.push(0);
        at
    };
    // Symbols: null, the section, the `$t` mapping symbol (Thumb code from 0), exports, then
    // the undefined symbols the relocations name.
    let mut symtab: Vec<u8> = vec![0; 16];
    let sym = |tab: &mut Vec<u8>, st_name: u32, value: u32, size: u32, info: u8, shndx: u16| {
        u32le(tab, st_name);
        u32le(tab, value);
        u32le(tab, size);
        tab.push(info);
        tab.push(0);
        u16le(tab, shndx);
    };
    sym(&mut symtab, 0, 0, 0, 3, 1); // STT_SECTION, local
                                     // The `$t` mapping symbol (Thumb code from 0) on ARM; another local section symbol keeps
                                     // the indices the same on Xtensa.
    if arm {
        let t = name("$t", &mut strtab);
        sym(&mut symtab, t, 0, 0, 0, 1);
    } else {
        sym(&mut symtab, 0, 0, 0, 3, 1);
    }
    let first_global = 3u32;
    for (export, at, size) in exports {
        let n = name(export, &mut strtab);
        // STB_GLOBAL | STT_FUNC; a Thumb function's address has bit 0 set.
        let value = if arm { *at as u32 | 1 } else { *at as u32 };
        sym(&mut symtab, n, value, *size as u32, 0x12, 1);
    }
    let mut undefined: Vec<String> = relocs.iter().filter_map(|r| r.symbol.clone()).collect();
    undefined.sort();
    undefined.dedup();
    let undefined_base = first_global + exports.len() as u32;
    for s in &undefined {
        let n = name(s, &mut strtab);
        sym(&mut symtab, n, 0, 0, 0x10, 0); // STB_GLOBAL, undefined
    }
    let mut rel = Vec::new();
    for r in relocs {
        // Symbol 1 is `.text`'s section symbol.
        let index = match &r.symbol {
            Some(s) => undefined_base + undefined.iter().position(|u| u == s).unwrap_or(0) as u32,
            None => 1,
        };
        u32le(&mut rel, r.at as u32);
        u32le(&mut rel, index << 8 | r.kind);
        if !arm {
            u32le(&mut rel, r.addend); // RELA addend
        }
    }
    let mut shstrtab = vec![0u8];
    let sh_text = name(".text", &mut shstrtab);
    let sh_rel = name(if arm { ".rel.text" } else { ".rela.text" }, &mut shstrtab);
    let sh_symtab = name(".symtab", &mut shstrtab);
    let sh_strtab = name(".strtab", &mut shstrtab);
    let sh_shstrtab = name(".shstrtab", &mut shstrtab);

    // Contents after the 52-byte header, each 4-aligned; section headers last.
    let mut body = Vec::new();
    let place = |data: &[u8], body: &mut Vec<u8>| -> u32 {
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        let at = 52 + body.len() as u32;
        body.extend_from_slice(data);
        at
    };
    let text_at = place(text, &mut body);
    let rel_at = place(&rel, &mut body);
    let symtab_at = place(&symtab, &mut body);
    let strtab_at = place(&strtab, &mut body);
    let shstrtab_at = place(&shstrtab, &mut body);
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    let shoff = 52 + body.len() as u32;

    let mut out = Vec::new();
    out.extend_from_slice(&[0x7F, b'E', b'L', b'F', 1, 1, 1, 0]);
    out.extend_from_slice(&[0; 8]);
    u16le(&mut out, 1); // ET_REL
    u16le(&mut out, if arm { 40 } else { 94 }); // EM_ARM / EM_XTENSA
    u32le(&mut out, 1);
    u32le(&mut out, 0); // entry
    u32le(&mut out, 0); // phoff
    u32le(&mut out, shoff);
    // ARM: EABI version 5, hard-float ABI. Xtensa: the flags GCC sets (0x300).
    u32le(&mut out, if arm { 0x0500_0400 } else { 0x300 });
    u16le(&mut out, 52);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, 40);
    u16le(&mut out, 6); // sections
    u16le(&mut out, 5); // .shstrtab
    out.extend_from_slice(&body);
    let section = |out: &mut Vec<u8>, f: [u32; 10]| {
        for x in f {
            u32le(out, x);
        }
    };
    section(&mut out, [0; 10]);
    section(
        &mut out,
        [sh_text, 1, 6, 0, text_at, text.len() as u32, 0, 0, 4, 0],
    );
    // SHT_REL (entries of 8 bytes) on ARM, SHT_RELA (12 bytes) on Xtensa.
    let (rel_type, rel_size) = if arm { (9, 8) } else { (4, 12) };
    section(
        &mut out,
        [
            sh_rel,
            rel_type,
            0x40,
            0,
            rel_at,
            rel.len() as u32,
            3,
            1,
            4,
            rel_size,
        ],
    );
    section(
        &mut out,
        [
            sh_symtab,
            2,
            0,
            0,
            symtab_at,
            symtab.len() as u32,
            4,
            first_global,
            4,
            16,
        ],
    );
    section(
        &mut out,
        [
            sh_strtab,
            3,
            0,
            0,
            strtab_at,
            strtab.len() as u32,
            0,
            0,
            1,
            0,
        ],
    );
    section(
        &mut out,
        [
            sh_shstrtab,
            3,
            0,
            0,
            shstrtab_at,
            shstrtab.len() as u32,
            0,
            0,
            1,
            0,
        ],
    );
    out
}
