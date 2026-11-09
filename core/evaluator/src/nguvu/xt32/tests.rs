//! The encoder's forms against GNU `as` for the ESP32 (`xtensa-esp32-elf-as`), word for word.

use super::*;

/// Words checked against `xtensa-esp32-elf-as` (wide forms; see the module doc).
#[test]
fn register_forms_match_the_assembler() {
    assert_eq!(add(2, 3, 4), 0x802340);
    assert_eq!(addx8(2, 3, 4), 0xb02340);
    assert_eq!(MEMW, 0x0020c0);
    assert_eq!(rsr(8, CCOUNT), 0x03ea80);
    assert_eq!(rsr(3, CCOUNT), 0x03ea30);
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
