//! Each target's object: its code units and export thunks laid out in `.text`, calls and
//! helpers relocated, and (ESP32) the stack each export takes.

use super::*;

/// Cortex-M: each export's thunk, then every function's code, `bl`s linked in place.
pub(super) fn object_t32(
    program: &BytecodeProgram,
    functions: &[usize],
) -> Result<Vec<u8>, String> {
    let direct = crate::nguvu::direct_entries(program, false);
    let units = units(
        Lengo::CortexM,
        program,
        &direct,
        functions.iter().map(|&i| (i, false)).collect(),
        codegen_t32::generate,
        |c: &codegen_t32::Code| c.calls.iter().map(|&(_, f)| f).collect(),
    )?;
    let mut text: Vec<u8> = Vec::new();
    let mut relocs: Vec<ElfReloc> = Vec::new();
    let mut symbols: Vec<(String, usize, usize)> = Vec::new();
    let mut thunk_calls: Vec<(usize, usize)> = Vec::new(); // (bl offset, export index)
    for &i in functions {
        let f = &program.functions[i];
        align(&mut text);
        let start = text.len();
        let (bytes, call_at) = thunk(f)?;
        thunk_calls.push((start + call_at, i));
        text.extend_from_slice(&bytes);
        symbols.push((format!("asili_{}", f.name), start, bytes.len()));
    }
    let mut placed: std::collections::BTreeMap<(usize, bool), usize> = Default::default();
    for (index, entry_direct, code) in &units {
        align(&mut text);
        let base = text.len();
        placed.insert((*index, *entry_direct), base);
        text.extend_from_slice(&code.bytes);
        for (at, symbol, kind) in &code.relocs {
            relocs.push(ElfReloc {
                at: base + at,
                symbol: Some(symbol.clone()),
                kind: *kind as u32,
                addend: 0,
            });
        }
    }
    for (index, entry_direct, code) in &units {
        let base = placed[&(*index, *entry_direct)];
        for &(at, callee) in &code.calls {
            let target = placed[&(callee as usize, true)];
            t32::link_bl(&mut text, base + at, target)?;
        }
    }
    for &(at, i) in &thunk_calls {
        t32::link_bl(&mut text, at, placed[&(i, false)])?;
    }
    Ok(elf_object(Lengo::CortexM, &text, &symbols, &relocs))
}

/// ESP32: each export's thunk, then every function (its literal pool, then its code). The
/// literals holding addresses are relocated: external functions by symbol, the program's own
/// functions against `.text` at their offset (calls reach any distance this way).
#[allow(clippy::type_complexity)]
pub(super) fn object_xt(
    program: &BytecodeProgram,
    functions: &[usize],
    kuu: Option<usize>,
) -> Result<(Vec<u8>, Vec<(String, u32)>), String> {
    // An export whose function has a direct entry is that entry with the C interface built in
    // (`export_interface`); failing that, a thunk calls the direct entry (lists in its register
    // buffer after the numbers, where the thunk puts them anyway), or else the ordinary one.
    let direct = crate::nguvu::direct_entries(program, true);
    let export_direct = |i: usize| direct.contains(&i);
    let mut own: std::collections::BTreeMap<usize, codegen_xt::Code> = Default::default();
    for &i in functions.iter().filter(|&&i| export_direct(i)) {
        let f = &program.functions[i];
        let ctx = lower::Ctx {
            program,
            direct: &direct,
            entry_direct: true,
            direct_lists: true,
            device: true,
        };
        let mut func = compile(Lengo::Esp32, i, &ctx).map_err(|e| kosa(Lengo::Esp32, f, &e))?;
        if export_interface(&mut func, f) {
            verify::verify(&func).map_err(|e| format!("kosa la ndani: {e}"))?;
            let code = codegen_xt::generate(&func).map_err(|e| kosa(Lengo::Esp32, f, &e))?;
            own.insert(i, code);
        }
    }
    let mut roots: Vec<(usize, bool)> = functions
        .iter()
        .filter(|i| !own.contains_key(i))
        .map(|&i| (i, export_direct(i)))
        .collect();
    roots.extend(
        own.values()
            .flat_map(|c| c.calls.iter().map(|&(_, f)| (f as usize, true))),
    );
    // The entry point runs `kuu`'s ordinary entry (it returns nothing, so it has no direct one).
    roots.extend(kuu.map(|k| (k, false)));
    roots.sort_unstable();
    roots.dedup();
    let units = units(
        Lengo::Esp32,
        program,
        &direct,
        roots,
        codegen_xt::generate,
        |c: &codegen_xt::Code| c.calls.iter().map(|&(_, f)| f).collect(),
    )?;
    let mut text: Vec<u8> = Vec::new();
    let mut relocs: Vec<ElfReloc> = Vec::new();
    let mut symbols: Vec<(String, usize, usize)> = Vec::new();
    // (literal offset in `text`, program function, whether its direct entry)
    let mut calls: Vec<(usize, usize, bool)> = Vec::new();
    // (name, export, whether its direct entry, frame) of each thunk
    let mut thunk_frames: Vec<(String, usize, bool, u32)> = Vec::new();
    // (literal offset in `text`, helper index)
    let mut helper_calls: Vec<(usize, u32)> = Vec::new();
    let place = |code: &codegen_xt::Code, text: &mut Vec<u8>, relocs: &mut Vec<ElfReloc>| {
        while !text.len().is_multiple_of(4) {
            text.push(0); // between functions: never executed
        }
        let base = text.len();
        text.extend_from_slice(&code.bytes);
        for (at, symbol) in &code.relocs {
            relocs.push(ElfReloc {
                at: base + at,
                symbol: Some(symbol.clone()),
                kind: R_XTENSA_32,
                addend: 0,
            });
        }
        base
    };
    for &i in functions {
        let f = &program.functions[i];
        if let Some(code) = own.get(&i) {
            let base = place(code, &mut text, &mut relocs);
            for &(at, callee) in &code.calls {
                calls.push((base + at, callee as usize, true));
            }
            for &(at, h) in &code.helpers {
                helper_calls.push((base + at, h));
            }
            symbols.push((
                format!("asili_{}", f.name),
                base + code.entry,
                code.bytes.len() - code.entry,
            ));
            continue;
        }
        let code = codegen_xt::thunk(&thunk_spec(f, export_direct(i)))
            .map_err(|e| kosa(Lengo::Esp32, f, &e))?;
        let base = place(&code, &mut text, &mut relocs);
        for &(at, _) in &code.calls {
            calls.push((base + at, i, export_direct(i)));
        }
        symbols.push((
            format!("asili_{}", f.name),
            base + code.entry,
            code.bytes.len() - code.entry,
        ));
        thunk_frames.push((f.name.clone(), i, export_direct(i), code.frame));
    }
    // `void app_main(void)`: zero `kuu`'s frame, run it, return.
    if let Some(k) = kuu {
        let f = &program.functions[k];
        // A program reading the board's clock starts it with the program.
        let clock = codegen_xt::helper_index(RtFn::BoardClock).expect("clock helper");
        let reads_clock = units
            .iter()
            .map(|(_, _, c)| c)
            .chain(own.values())
            .any(|c| c.helpers.iter().any(|&(_, h)| h == clock));
        let spec = codegen_xt::ThunkSpec {
            params: Vec::new(),
            returns: Vec::new(),
            entry_point: true,
            start_clock: reads_clock.then_some(clock),
            ..thunk_spec(f, false)
        };
        let code = codegen_xt::thunk(&spec).map_err(|e| kosa(Lengo::Esp32, f, &e))?;
        let base = place(&code, &mut text, &mut relocs);
        for &(at, _) in &code.calls {
            calls.push((base + at, k, false));
        }
        for &(at, h) in &code.helpers {
            helper_calls.push((base + at, h));
        }
        symbols.push((
            "app_main".to_string(),
            base + code.entry,
            code.bytes.len() - code.entry,
        ));
        thunk_frames.push(("kuu".to_string(), k, false, code.frame));
    }
    let mut placed: std::collections::BTreeMap<(usize, bool), usize> = Default::default();
    for (index, entry_direct, code) in &units {
        let base = place(code, &mut text, &mut relocs);
        placed.insert((*index, *entry_direct), base + code.entry);
        for &(at, callee) in &code.calls {
            calls.push((base + at, callee as usize, true));
        }
        for &(at, h) in &code.helpers {
            helper_calls.push((base + at, h));
        }
    }
    // The helpers the code calls, compiled from IR once each.
    let mut helpers: std::collections::BTreeMap<u32, usize> = Default::default();
    let mut helper_frames: std::collections::BTreeMap<u32, u32> = Default::default();
    let used: std::collections::BTreeSet<u32> = helper_calls.iter().map(|&(_, h)| h).collect();
    for h in used {
        let mut func = helper_func(codegen_xt::HELPERS[h as usize]);
        verify::verify(&func).map_err(|e| format!("kosa la ndani: {e}"))?;
        opt::optimize_with(&mut func, false).map_err(|e| format!("kosa la ndani: {e}"))?;
        verify::verify(&func).map_err(|e| format!("kosa la ndani: {e}"))?;
        let code = codegen_xt::generate(&func)?;
        let base = place(&code, &mut text, &mut relocs);
        helpers.insert(h, base + code.entry);
        helper_frames.insert(h, code.frame);
    }
    for (at, h) in helper_calls {
        relocs.push(ElfReloc {
            at,
            symbol: None,
            kind: R_XTENSA_32,
            addend: helpers[&h] as u32,
        });
    }
    for (at, callee, direct) in calls {
        relocs.push(ElfReloc {
            at,
            symbol: None,
            kind: R_XTENSA_32,
            addend: placed[&(callee, direct)] as u32,
        });
    }
    // Stack: frames along the deepest call chain (strict code has no recursion).
    let unit_of: std::collections::BTreeMap<(usize, bool), &codegen_xt::Code> =
        units.iter().map(|(i, d, c)| ((*i, *d), c)).collect();
    fn depth(
        at: (usize, bool),
        units: &std::collections::BTreeMap<(usize, bool), &codegen_xt::Code>,
        helper_frames: &std::collections::BTreeMap<u32, u32>,
        memo: &mut std::collections::BTreeMap<(usize, bool), u32>,
        level: usize,
    ) -> Result<u32, String> {
        if let Some(&d) = memo.get(&at) {
            return Ok(d);
        }
        if level > 1000 {
            return Err("kosa la ndani: mfululizo wa miito hauishi".into());
        }
        let code = units[&at];
        let mut deepest = 0;
        for &(_, callee) in &code.calls {
            deepest = deepest.max(depth(
                (callee as usize, true),
                units,
                helper_frames,
                memo,
                level + 1,
            )?);
        }
        for &(_, h) in &code.helpers {
            deepest = deepest.max(helper_frames[&h]);
        }
        let d = code.frame + deepest;
        memo.insert(at, d);
        Ok(d)
    }
    let mut memo = Default::default();
    let mut stack = Vec::new();
    for &i in functions.iter().chain(&kuu) {
        let name = &program.functions[i].name;
        if let Some(code) = own.get(&i) {
            let mut deepest = 0;
            for &(_, callee) in &code.calls {
                let d = depth(
                    (callee as usize, true),
                    &unit_of,
                    &helper_frames,
                    &mut memo,
                    1,
                )?;
                deepest = deepest.max(d);
            }
            for &(_, h) in &code.helpers {
                deepest = deepest.max(helper_frames[&h]);
            }
            stack.push((name.clone(), code.frame + deepest));
        } else if let Some((_, index, direct, frame)) =
            thunk_frames.iter().find(|(_, index, _, _)| *index == i)
        {
            let d = depth((*index, *direct), &unit_of, &helper_frames, &mut memo, 0)?;
            stack.push((name.clone(), frame + d));
        }
    }
    Ok((elf_object(Lengo::Esp32, &text, &symbols, &relocs), stack))
}

pub(super) fn align(text: &mut Vec<u8>) {
    while !text.len().is_multiple_of(4) {
        text.extend_from_slice(&0xBF00u16.to_le_bytes()); // nop
    }
}

/// The export of `f`: builds `f`'s frame on the stack from `hoja` (r0), calls its ordinary
/// entry, and stores the result at `matokeo` (r1). Returns the code and the offset of the `bl`
/// to link to the entry.
pub(super) fn thunk(f: &BytecodeFunc) -> Result<(Vec<u8>, usize), String> {
    let nums = 8 * f.num_regs;
    let lists = 8 * f.list_regs;
    let host = nums + lists; // depth, runtime table, stack limit: 24 bytes, all zero
    let size = (host + 24).div_ceil(8) * 8;
    let mut a = Asm::new();
    a.push(0x40F8); // r3-r7, lr (six words: `sp` stays 8-byte aligned)
    a.mov(4, 0);
    a.mov(5, 1);
    a.adjust_sp(size, true, 12);
    // Zero the whole frame: registers, list references, host fields.
    a.mov_imm(0, 0);
    a.mov_imm(1, 0);
    for off in (0..size).step_by(8) {
        store_pair(&mut a, 0, 1, off);
    }
    // Constants the registers start with.
    for &(reg, value) in &f.num_consts {
        let bits = value.to_bits();
        a.mov_imm(0, bits as u32);
        a.mov_imm(1, (bits >> 32) as u32);
        store_pair(&mut a, 0, 1, 8 * reg);
    }
    // Arguments, eight bytes each in `hoja`.
    for (i, p) in f.params.iter().enumerate() {
        let from = 8 * i as u32;
        if from > 1020 {
            return Err("ina hoja nyingi mno".into());
        }
        a.ldrd(0, 1, 4, from);
        let to = if p.ty == Ty::List {
            nums + 8 * p.reg
        } else {
            8 * p.reg
        };
        store_pair(&mut a, 0, 1, to);
    }
    // entry(runtime: unused on a device, host, frame = list references, nums)
    a.mov_imm(0, 0);
    a.add_any(1, SP, host);
    a.add_any(2, SP, nums);
    a.addw(3, SP, 0);
    let call_at = a.bl_placeholder();
    // r1 = status kind, r0 = the pc of the `Return` reached.
    let fail = a.new_label();
    let done = a.new_label();
    a.cmp_imm(1, STATUS_RETURN);
    a.b_cond(Cond::Ne, fail);
    for (pc, op) in f.code.iter().enumerate() {
        let Opcode::Return { src } = op else {
            continue;
        };
        if !matches!(src.ty, Ty::Num | Ty::Bool) {
            continue;
        }
        let next = a.new_label();
        a.mov_imm(12, pc as u32);
        a.cmp(0, 12);
        a.b_cond(Cond::Ne, next);
        let off = 8 * src.reg;
        if off <= 1020 {
            a.vldr(0, SP, off);
        } else {
            a.add_any(12, SP, off);
            a.vldr(0, 12, 0);
        }
        a.vstr(0, 5, 0);
        a.b(done);
        a.bind(next);
    }
    a.bind(done);
    a.mov_imm(0, 0);
    let out = a.new_label();
    a.b(out);
    a.bind(fail);
    a.mov(0, 1);
    a.bind(out);
    a.adjust_sp(size, false, 12);
    a.pop(0x80F8); // r3-r7, pc
    let (bytes, _) = a.finish()?;
    Ok((bytes, call_at))
}

pub(super) fn store_pair(a: &mut Asm, lo: u8, hi: u8, off: u32) {
    if off <= 1020 {
        a.strd(lo, hi, SP, off);
    } else {
        a.add_any(12, SP, off);
        a.strd(lo, hi, 12, 0);
    }
}
