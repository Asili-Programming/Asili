//! The device targets: a program's strict (`#[salama]`) functions compiled to machine code as
//! an ELF relocatable object a firmware links, plus a C header declaring them
//! (`pata jenga --lengo cortex-m` / `--lengo esp32`). Cortex-M code comes from
//! [`super::codegen_t32`] (Thumb-2), ESP32 code from [`super::codegen_xt`] (Xtensa LX6).
//!
//! Each strict function `f` is exported as
//! `int32_t asili_f(const asili_f_hoja *hoja, double *matokeo)`: `hoja` holds its arguments in
//! order (a `Namba` or `Ukweli` as a `double`, an `Orodha<Namba>` as `asili_orodha`, a pointer
//! to the caller's `double` array and its length, eight bytes each); the result is stored at
//! `matokeo`, and the return value is 0, or non-zero when the call failed (an index out of range,
//! say) — the firmware's cue to enter its safe state. The export is a small thunk that builds
//! the function's register frame on the stack and calls its native code; strict functions call
//! each other directly. The code calls into the device runtime (`asili_kifaa_*`: list access,
//! `fmod`, `pow`, conversions) and the compiler runtime's 64-bit and soft-float helpers, all
//! resolved when the firmware is linked.
//!
//! Only what a device can run without a host is accepted: numbers, truth values and the caller's
//! numeric lists, with no allocation — anything else in a strict function is a build error.
//! Everything up to the IR (checks, inlining, lowering, optimization) is shared by the targets.

use super::codegen_t32;
use super::codegen_xt;
use super::ir::{BlockData, Class, FCond, FloatOp, Func, ICond, Inst, IntOp, RtFn, Term, VReg};
use super::t32::{self, Asm, Cond, SP};
use super::{lower, opt, verify};
use crate::bytecode::{BytecodeFunc, BytecodeProgram, Opcode, Operand, Ty};
use crate::numlist::Kind;

/// A device target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lengo {
    /// ARMv7E-M with the `fpv5-d16` double-precision unit (hard-float ABI).
    CortexM,
    /// ESP32: Xtensa LX6, windowed ABI, doubles in software.
    Esp32,
}

impl Lengo {
    /// The name in messages and file names.
    pub fn jina(self) -> &'static str {
        match self {
            Lengo::CortexM => "Cortex-M",
            Lengo::Esp32 => "ESP32",
        }
    }

    /// Whether a device can provide runtime function `target` (on ESP32 most are inlined by
    /// [`inline_runtime`], the rest come from the C library).
    fn runtime_symbol(self, target: RtFn) -> Option<&'static str> {
        match self {
            Lengo::CortexM => codegen_t32::runtime_symbol(target),
            Lengo::Esp32 => codegen_t32::runtime_symbol(target)
                .map(|_| codegen_xt::runtime_symbol(target).unwrap_or("")),
        }
    }
}

/// `R_XTENSA_32`: a 32-bit absolute address (the literals naming external functions).
const R_XTENSA_32: u32 = 1;

/// A built device object and its header.
pub struct Kifaa {
    /// ELF32 relocatable object (ARM EABI5 hard float, or Xtensa).
    pub object: Vec<u8>,
    /// C declarations of the exports.
    pub header: String,
    /// Exported function names, in order.
    pub exports: Vec<String>,
    /// For each export (ESP32), the most stack a call takes in Asili's code: the export's frame
    /// plus the deepest chain of frames through the functions and helpers it calls (the C
    /// library's and the compiler runtime's own frames not included). Empty for Cortex-M.
    pub stack: Vec<(String, u32)>,
}

/// Status kinds in the high word of a function's return value (`native::STATUS_*`).
const STATUS_RETURN: u32 = 3;

/// Build `functions` (strict functions of `program`, by index) for Cortex-M; `jina` names the
/// header guard. Errors name the function and why it cannot run on a device.
pub fn build(program: &BytecodeProgram, functions: &[usize], jina: &str) -> Result<Kifaa, String> {
    build_for(Lengo::CortexM, program, functions, jina)
}

/// Build `functions` for `lengo`.
pub fn build_for(
    lengo: Lengo,
    program: &BytecodeProgram,
    functions: &[usize],
    jina: &str,
) -> Result<Kifaa, String> {
    for &i in functions {
        let f = &program.functions[i];
        signature(f).map_err(|e| kosa(lengo, f, &e))?;
    }
    // A strict function calling one that takes a list would go through the host: on a device
    // the callee's body is inlined instead (strict code has no recursion, so this ends).
    let mut inlined = program.clone();
    for &i in functions {
        inline_list_calls(&mut inlined, i).map_err(|e| kosa(lengo, &program.functions[i], &e))?;
    }
    crate::bytecode_verify::verify(&inlined).map_err(|e| format!("kosa la ndani: {e}"))?;
    let program = &inlined;
    let (object, stack) = match lengo {
        Lengo::CortexM => (object_t32(program, functions)?, Vec::new()),
        Lengo::Esp32 => object_xt(program, functions)?,
    };
    Ok(Kifaa {
        object,
        stack,
        header: header(lengo, program, functions, jina),
        exports: functions
            .iter()
            .map(|&i| program.functions[i].name.clone())
            .collect(),
    })
}

/// The entries in `roots` (function, whether its direct entry), then the direct entries they
/// reach, each lowered and generated by `generate`; `calls` lists the program functions a
/// generated unit calls directly.
fn units<C>(
    lengo: Lengo,
    program: &BytecodeProgram,
    direct: &asili_parser::FxHashSet<usize>,
    roots: Vec<(usize, bool)>,
    generate: impl Fn(&Func) -> Result<C, String>,
    calls: impl Fn(&C) -> Vec<u32>,
) -> Result<Vec<(usize, bool, C)>, String> {
    let mut units: Vec<(usize, bool, C)> = Vec::new();
    let mut queue: Vec<(usize, bool)> = roots;
    let mut seen: std::collections::BTreeSet<(usize, bool)> = queue.iter().copied().collect();
    while let Some((index, entry_direct)) = queue.pop() {
        let f = &program.functions[index];
        let ctx = lower::Ctx {
            program,
            direct,
            entry_direct,
            direct_lists: lengo == Lengo::Esp32,
        };
        let func = compile(lengo, index, &ctx).map_err(|e| kosa(lengo, f, &e))?;
        let code = generate(&func).map_err(|e| kosa(lengo, f, &e))?;
        for callee in calls(&code) {
            if seen.insert((callee as usize, true)) {
                queue.push((callee as usize, true));
            }
        }
        units.push((index, entry_direct, code));
    }
    units.sort_by_key(|(i, d, _)| (*d, *i));
    Ok(units)
}

/// Cortex-M: each export's thunk, then every function's code, `bl`s linked in place.
fn object_t32(program: &BytecodeProgram, functions: &[usize]) -> Result<Vec<u8>, String> {
    let direct = super::direct_entries(program, false);
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
fn object_xt(
    program: &BytecodeProgram,
    functions: &[usize],
) -> Result<(Vec<u8>, Vec<(String, u32)>), String> {
    // An export whose function has a direct entry is that entry with the C interface built in
    // (`export_interface`); failing that, a thunk calls the direct entry (lists in its register
    // buffer after the numbers, where the thunk puts them anyway), or else the ordinary one.
    let direct = super::direct_entries(program, true);
    let export_direct = |i: usize| direct.contains(&i);
    let mut own: std::collections::BTreeMap<usize, codegen_xt::Code> = Default::default();
    for &i in functions.iter().filter(|&&i| export_direct(i)) {
        let f = &program.functions[i];
        let ctx = lower::Ctx {
            program,
            direct: &direct,
            entry_direct: true,
            direct_lists: true,
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
    for &i in functions {
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

/// Build an export's C interface — `int32_t asili_f(const hoja *h, double *matokeo)` — into the
/// IR of `f`'s direct entry, so the export is that function with no thunk in between: its
/// registers' reads become reads of the arguments in `hoja` (eight bytes each, list references
/// included, in parameter order), its number is stored at `matokeo` with 0 returned, and a
/// failing status returns its kind (non-zero). `false` (and `func` to be dropped) when the
/// entry uses its register buffer otherwise, or the host fields.
fn export_interface(func: &mut Func, f: &BytecodeFunc) -> bool {
    // Offset in the direct entry's register buffer → offset in `hoja`.
    let mut from: std::collections::HashMap<i32, i32> = Default::default();
    for (i, p) in f.params.iter().enumerate() {
        let reg = p.reg as i64;
        let at = if p.ty == Ty::List {
            8 * (f.num_regs as i64 + reg)
        } else {
            8 * reg
        };
        let (Ok(at), Ok(to)) = (i32::try_from(at), i32::try_from(8 * i as i64)) else {
            return false;
        };
        from.insert(at, to);
    }
    let buffer = |v: &VReg| *v == super::ir::NUMS || *v == super::ir::HOST;
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            if let Inst::Load { base, offset, .. } = inst {
                if *base == super::ir::NUMS {
                    let Some(&to) = from.get(offset) else {
                        return false;
                    };
                    *offset = to;
                    continue;
                }
            }
            if inst.uses().iter().any(buffer) {
                return false;
            }
        }
        if block.term.uses().iter().any(buffer) {
            return false;
        }
    }
    for b in 0..func.blocks.len() {
        let returned = match func.blocks[b].term {
            Term::ReturnNum(v) => {
                func.blocks[b].insts.push(Inst::Store {
                    src: v,
                    base: super::ir::HOST, // `matokeo`
                    offset: 0,
                });
                let zero = func.new_vreg(Class::Int);
                func.blocks[b].insts.push(Inst::IConst {
                    dst: zero,
                    value: 0,
                });
                zero
            }
            Term::Return(status) => {
                let kind = func.new_vreg(Class::Int);
                func.blocks[b].insts.push(Inst::IntImm {
                    op: IntOp::Sar,
                    dst: kind,
                    a: status,
                    imm: 32,
                });
                kind
            }
            _ => continue,
        };
        func.blocks[b].term = Term::Return(returned);
    }
    func.args = super::ir::EXPORT_ARGS;
    true
}

/// What the ESP32 export thunk of `f` needs (see [`codegen_xt::thunk`]).
fn thunk_spec(f: &BytecodeFunc, direct: bool) -> codegen_xt::ThunkSpec {
    codegen_xt::ThunkSpec {
        direct,
        num_regs: f.num_regs,
        list_regs: f.list_regs,
        num_consts: f
            .num_consts
            .iter()
            .map(|&(reg, value)| (reg, value.to_bits()))
            .collect(),
        params: f.params.iter().map(|p| (p.reg, p.ty == Ty::List)).collect(),
        returns: f
            .code
            .iter()
            .enumerate()
            .filter_map(|(pc, op)| match op {
                Opcode::Return { src } if matches!(src.ty, Ty::Num | Ty::Bool) => {
                    Some((pc as u32, src.reg))
                }
                _ => None,
            })
            .collect(),
    }
}

/// Inline, into function `caller`, every call to a function taking a list, until none is left.
fn inline_list_calls(program: &mut BytecodeProgram, caller: usize) -> Result<(), String> {
    for _ in 0..1000 {
        let found =
            program.functions[caller]
                .code
                .iter()
                .enumerate()
                .find_map(|(pc, op)| match op {
                    Opcode::Call(call)
                        if program.functions[call.function as usize]
                            .params
                            .iter()
                            .any(|p| p.ty == Ty::List) =>
                    {
                        Some((pc, (**call).clone()))
                    }
                    _ => None,
                });
        let Some((pc, call)) = found else {
            return Ok(());
        };
        let callee = program.functions[call.function as usize].clone();
        inline_at(&mut program.functions[caller], pc, &call, &callee)?;
    }
    Err("miito iliyo ndani ya miito ni mingi mno".into())
}

/// Replace the call at `pc` of `f` with `callee`'s body: its registers renamed past `f`'s,
/// its parameters bound to the arguments (a list parameter shares the caller's list, so the
/// callee must not write it), and each return a move into the call's destination and a jump
/// past the body.
fn inline_at(
    f: &mut BytecodeFunc,
    pc: usize,
    call: &crate::bytecode::CallOp,
    callee: &BytecodeFunc,
) -> Result<(), String> {
    let (num_base, list_base) = (f.num_regs, f.list_regs);
    let num = |r: u32| r + num_base;
    let written_lists: std::collections::BTreeSet<u32> = callee
        .code
        .iter()
        .flat_map(crate::native::list_writes)
        .chain(callee.code.iter().filter_map(|op| match op {
            Opcode::ListSet { list, .. } => Some(*list),
            _ => None,
        }))
        .collect();
    let mut list_map: std::collections::HashMap<u32, u32> = Default::default();
    let mut prologue = Vec::new();
    for (p, arg) in callee.params.iter().zip(&call.args) {
        match p.ty {
            Ty::List => {
                if written_lists.contains(&p.reg) {
                    return Err(format!(
                        "inaita '{}', inayoandika kwenye orodha iliyopokelewa",
                        callee.name
                    ));
                }
                list_map.insert(p.reg, arg.reg);
            }
            Ty::Num | Ty::Bool => prologue.push(Opcode::Mov {
                dst: num(p.reg),
                src: arg.reg,
            }),
            Ty::Val => return Err(format!("inaita '{}' kwa hoja isiyo ya namba", callee.name)),
        }
    }
    let list = |r: u32| list_map.get(&r).copied().unwrap_or(r + list_base);
    // The callee's constants: never-written ones join the caller's; others are set on entry
    // (each call starts with them), from a fresh constant register.
    let callee_writes: std::collections::BTreeSet<u32> =
        callee.code.iter().flat_map(lower::writes).collect();
    let mut extra_regs = 0;
    for &(r, v) in &callee.num_consts {
        if callee_writes.contains(&r) {
            let k = num_base + callee.num_regs + extra_regs;
            extra_regs += 1;
            f.num_consts.push((k, v));
            prologue.push(Opcode::Mov {
                dst: num(r),
                src: k,
            });
        } else {
            f.num_consts.push((num(r), v));
        }
    }
    // New positions of the callee's instructions (a return becomes a move and a jump).
    let start = pc + prologue.len();
    let mut at = Vec::with_capacity(callee.code.len() + 1);
    let mut next = start;
    for op in &callee.code {
        at.push(next);
        next += match op {
            Opcode::Return { src } if matches!(src.ty, Ty::Num | Ty::Bool) => 2,
            _ => 1,
        };
    }
    at.push(next);
    let after = next;
    let target = |t: u32| at[t as usize] as u32;
    let operand = |o: &Operand| -> Result<Operand, String> {
        Ok(Operand {
            ty: o.ty,
            reg: match o.ty {
                Ty::Num | Ty::Bool => num(o.reg),
                Ty::List => list(o.reg),
                Ty::Val => return Err("hoja isiyo ya namba".into()),
            },
        })
    };
    let mut body = Vec::with_capacity(after - pc);
    body.extend(prologue);
    for op in &callee.code {
        use Opcode as O;
        let renamed = match op {
            O::Mov { dst, src } => O::Mov {
                dst: num(*dst),
                src: num(*src),
            },
            O::Add { dst, a, b } => O::Add {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Sub { dst, a, b } => O::Sub {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Mul { dst, a, b } => O::Mul {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Div { dst, a, b } => O::Div {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Rem { dst, a, b } => O::Rem {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Pow { dst, a, b } => O::Pow {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::BitAnd { dst, a, b } => O::BitAnd {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::BitOr { dst, a, b } => O::BitOr {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::BitXor { dst, a, b } => O::BitXor {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Shl { dst, a, b } => O::Shl {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Shr { dst, a, b } => O::Shr {
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Neg { dst, src } => O::Neg {
                dst: num(*dst),
                src: num(*src),
            },
            O::BitNot { dst, src } => O::BitNot {
                dst: num(*dst),
                src: num(*src),
            },
            O::Not { dst, src } => O::Not {
                dst: num(*dst),
                src: num(*src),
            },
            O::Floor { dst, src } => O::Floor {
                dst: num(*dst),
                src: num(*src),
            },
            O::Ceil { dst, src } => O::Ceil {
                dst: num(*dst),
                src: num(*src),
            },
            O::Trunc { dst, src } => O::Trunc {
                dst: num(*dst),
                src: num(*src),
            },
            O::Cmp { op, dst, a, b } => O::Cmp {
                op: *op,
                dst: num(*dst),
                a: num(*a),
                b: num(*b),
            },
            O::Jump { target: t } => O::Jump { target: target(*t) },
            O::JumpIfFalse { cond, target: t } => O::JumpIfFalse {
                cond: num(*cond),
                target: target(*t),
            },
            O::JumpIfTrue { cond, target: t } => O::JumpIfTrue {
                cond: num(*cond),
                target: target(*t),
            },
            O::JumpIfNot {
                op,
                a,
                b,
                target: t,
            } => O::JumpIfNot {
                op: *op,
                a: num(*a),
                b: num(*b),
                target: target(*t),
            },
            O::ForStep {
                ctr,
                end,
                target: t,
            } => O::ForStep {
                ctr: num(*ctr),
                end: num(*end),
                target: target(*t),
            },
            O::ListGet {
                dst,
                list: l,
                idx,
                mode,
            } => O::ListGet {
                dst: num(*dst),
                list: list(*l),
                idx: num(*idx),
                mode: *mode,
            },
            O::ListSet { list: l, idx, src } => O::ListSet {
                list: list(*l),
                idx: num(*idx),
                src: num(*src),
            },
            O::ListLen { dst, list: l } => O::ListLen {
                dst: num(*dst),
                list: list(*l),
            },
            O::ListMov { dst, src } => O::ListMov {
                dst: list(*dst),
                src: list(*src),
            },
            O::CheckDepth => O::CheckDepth,
            O::Call(c) => O::Call(Box::new(crate::bytecode::CallOp {
                function: c.function,
                args: c.args.iter().map(operand).collect::<Result<_, _>>()?,
                dst: if c.dst.ty == Ty::Val {
                    c.dst
                } else {
                    operand(&c.dst)?
                },
            })),
            O::Return { src } if matches!(src.ty, Ty::Num | Ty::Bool) => {
                if matches!(call.dst.ty, Ty::Num | Ty::Bool) {
                    body.push(O::Mov {
                        dst: call.dst.reg,
                        src: num(src.reg),
                    });
                } else {
                    body.push(O::Jump {
                        target: (body.len() + pc + 1) as u32,
                    });
                }
                O::Jump {
                    target: after as u32,
                }
            }
            O::ReturnTupu => O::Jump {
                target: after as u32,
            },
            other => {
                return Err(format!(
                    "agizo `{}` ndani ya '{}'",
                    other.name(),
                    callee.name
                ))
            }
        };
        body.push(renamed);
    }
    debug_assert_eq!(pc + body.len(), after);
    // The caller's jumps past the call move by the body's extra length.
    let grow = (body.len() as u32).saturating_sub(1);
    for op in f.code.iter_mut() {
        if let Opcode::Jump { target: t }
        | Opcode::JumpIfFalse { target: t, .. }
        | Opcode::JumpIfTrue { target: t, .. }
        | Opcode::JumpIfNot { target: t, .. }
        | Opcode::ForStep { target: t, .. } = op
        {
            if *t as usize > pc {
                *t += grow;
            }
        }
    }
    f.code.splice(pc..pc + 1, body);
    f.num_regs += callee.num_regs + extra_regs;
    f.list_regs += callee.list_regs;
    Ok(())
}

fn kosa(lengo: Lengo, f: &BytecodeFunc, e: &str) -> String {
    format!(
        "kazi salama '{}' haiwezi kujengwa kwa {}: {e}",
        f.name,
        lengo.jina()
    )
}

fn align(text: &mut Vec<u8>) {
    while !text.len().is_multiple_of(4) {
        text.extend_from_slice(&0xBF00u16.to_le_bytes()); // nop
    }
}

/// The parameters and result a device export can pass.
fn signature(f: &BytecodeFunc) -> Result<(), String> {
    if !f
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("jina lake si jina halali la C".into());
    }
    for p in &f.params {
        if !matches!(p.ty, Ty::Num | Ty::Bool | Ty::List) {
            return Err("hoja zake ziwe Namba, Ukweli au Orodha<Namba> tu".into());
        }
    }
    Ok(())
}

/// Lower and optimize one entry (without instructions a device lacks), ready for a target's
/// code generator.
fn compile(lengo: Lengo, index: usize, ctx: &lower::Ctx) -> Result<Func, String> {
    let function = &ctx.program.functions[index];
    let mut func =
        lower::lower(index, function, ctx).ok_or("haikuweza kutafsiriwa kuwa msimbo wa mashine")?;
    check_lists(&func)?;
    check_host_calls(lengo, &func, function)?;
    if lengo == Lengo::Esp32 {
        inline_runtime(&mut func)?;
    }
    verify::verify(&func).map_err(|e| format!("kosa la ndani: {e}"))?;
    opt::optimize_with(&mut func, false).map_err(|e| format!("kosa la ndani: {e}"))?;
    verify::verify(&func).map_err(|e| format!("kosa la ndani: {e}"))?;
    Ok(func)
}

/// Status of a call that cannot be made on a device (`native::STATUS_FAIL << 32`).
const STATUS_FAIL: i64 = 2 << 32;
/// What `exec` returns on a device: the call fails (`native::STATUS_FAIL`); it is only reached
/// on an error path (see [`check_host_calls`]).
const EXEC_FAIL: i64 = 2;

/// Replace the device runtime's small calls with IR, so the ESP32 needs no runtime library of its
/// own: list access becomes loads from the caller's list references (`{ double *data; uint32_t
/// len }`, eight bytes on these 32-bit devices) and the host-only calls their failing status. The
/// float conversions stay calls, to helpers compiled from IR once per object
/// ([`helper_func`]); only `fmod` and `pow` come from the C library. Each computes what
/// `driver/kifaa` computes, which is what the host's runtime computes.
fn inline_runtime(func: &mut Func) -> Result<(), String> {
    let consts = iconsts(func);
    let mut classes = std::mem::take(&mut func.classes);
    for block in &mut func.blocks {
        let mut out = Vec::with_capacity(block.insts.len());
        for inst in std::mem::take(&mut block.insts) {
            let Inst::Call {
                target, args, dst, ..
            } = &inst
            else {
                out.push(inst);
                continue;
            };
            let mut e = Emit {
                classes: &mut classes,
                out: &mut out,
            };
            match (target, dst) {
                (RtFn::Exec, Some(d)) => e.push(Inst::IConst {
                    dst: *d,
                    value: EXEC_FAIL,
                }),
                (RtFn::CallHost | RtFn::DepthError, Some(d)) => e.push(Inst::IConst {
                    dst: *d,
                    value: STATUS_FAIL,
                }),
                (RtFn::Exec | RtFn::CallHost | RtFn::DepthError, None) => {}
                (RtFn::ListPtr | RtFn::ListLen, Some(d)) => {
                    let reg = args
                        .get(1)
                        .and_then(|r| consts.get(&r.0))
                        .ok_or("kosa la ndani: rejista ya orodha si thabiti")?;
                    let offset =
                        i32::try_from(8 * reg).map_err(|_| "kosa la ndani: orodha nyingi mno")?;
                    // Both words of the reference: data pointer low, length high.
                    let both = e.int();
                    e.push(Inst::Load {
                        dst: both,
                        base: args[0],
                        offset,
                    });
                    let word = if *target == RtFn::ListLen {
                        let w = e.int();
                        e.push(Inst::IntImm {
                            op: IntOp::Sar,
                            dst: w,
                            a: both,
                            imm: 32,
                        });
                        w
                    } else {
                        both
                    };
                    let mask = e.iconst(0xFFFF_FFFF);
                    e.push(Inst::Int {
                        op: IntOp::And,
                        dst: *d,
                        a: word,
                        b: mask,
                    });
                }
                _ => out.push(inst),
            }
        }
        block.insts = out;
    }
    func.classes = classes;
    Ok(())
}

/// The IR function for helper `target` (one of [`codegen_xt::HELPERS`]): its 64-bit argument
/// arrives as two words ([`super::ir::HELPER_ARGS`]), its 64-bit result (a double's bits or an
/// integer) is returned.
fn helper_func(target: RtFn) -> Func {
    let mut classes = vec![Class::Int, Class::Int];
    let mut out = Vec::new();
    let mut e = Emit {
        classes: &mut classes,
        out: &mut out,
    };
    let (lo, hi) = (VReg(0), VReg(1));
    let high = e.int();
    e.push(Inst::IntImm {
        op: IntOp::Shl,
        dst: high,
        a: hi,
        imm: 32,
    });
    let bits = e.int();
    e.push(Inst::Int {
        op: IntOp::Or,
        dst: bits,
        a: high,
        b: lo,
    });
    let x = e.float();
    e.push(Inst::BitsFloat { dst: x, src: bits });
    let result = match target {
        RtFn::Floor | RtFn::Ceil => {
            let r = e.floor_ceil(x, target == RtFn::Floor);
            let b = e.int();
            e.push(Inst::FloatBits { dst: b, src: r });
            b
        }
        RtFn::FloatToIntSat => e.float_to_int_sat(x),
        _ => e.shift_amount(x),
    };
    Func {
        classes,
        blocks: vec![BlockData {
            insts: out,
            term: Term::Return(result),
        }],
        cold: vec![false],
        call_buffer: 0,
        args: super::ir::HELPER_ARGS,
    }
}

/// Appends instructions (and new registers) for [`inline_runtime`] and [`helper_func`].
struct Emit<'a> {
    classes: &'a mut Vec<Class>,
    out: &'a mut Vec<Inst>,
}

impl Emit<'_> {
    fn push(&mut self, inst: Inst) {
        self.out.push(inst);
    }

    fn fresh(&mut self, class: Class) -> VReg {
        self.classes.push(class);
        VReg(self.classes.len() as u32 - 1)
    }

    fn int(&mut self) -> VReg {
        self.fresh(Class::Int)
    }

    fn float(&mut self) -> VReg {
        self.fresh(Class::Float)
    }

    fn iconst(&mut self, value: i64) -> VReg {
        let dst = self.int();
        self.push(Inst::IConst { dst, value });
        dst
    }

    fn fconst(&mut self, value: f64) -> VReg {
        let dst = self.float();
        self.push(Inst::FConst { dst, value });
        dst
    }

    fn fcmp(&mut self, cond: FCond, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.push(Inst::FCmp { cond, dst, a, b });
        dst
    }

    fn select_int(&mut self, cond: VReg, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.push(Inst::Select { dst, cond, a, b });
        dst
    }

    /// `floor(x)` or `ceil(x)`: `t - 1` for a negative non-integer, `t + 1` for a positive one,
    /// with `t = trunc(x)` — exact, since |t| < 2^52 whenever x is not an integer.
    fn floor_ceil(&mut self, x: VReg, floor: bool) -> VReg {
        let t = self.trunc(x);
        let zero = self.fconst(0.0);
        let one = self.fconst(1.0);
        let (cond, op) = if floor {
            (FCond::Olt, FloatOp::Sub)
        } else {
            (FCond::Ogt, FloatOp::Add)
        };
        let side = self.fcmp(cond, x, zero);
        let inexact = self.fcmp(FCond::Une, t, x);
        let step = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: step,
            a: side,
            b: inexact,
        });
        let moved = self.float();
        self.push(Inst::Float {
            op,
            dst: moved,
            a: t,
            b: one,
        });
        let r = self.float();
        self.push(Inst::Select {
            dst: r,
            cond: step,
            a: moved,
            b: t,
        });
        r
    }

    /// Rust's saturating `x as i64`: NaN -> 0, beyond the range -> MIN / MAX.
    fn float_to_int_sat(&mut self, x: VReg) -> VReg {
        let t = self.trunc(x);
        let i = self.int();
        self.push(Inst::FloatToInt { dst: i, src: t });
        let limit = self.fconst(9223372036854775808.0);
        let low = self.fconst(-9223372036854775808.0);
        let above = self.fcmp(FCond::Oge, x, limit);
        let below = self.fcmp(FCond::Ole, x, low);
        let nan = self.fcmp(FCond::Une, x, x);
        let max = self.iconst(i64::MAX);
        let min = self.iconst(i64::MIN);
        let zero = self.iconst(0);
        let r1 = self.select_int(above, max, i);
        let r2 = self.select_int(below, min, r1);
        self.select_int(nan, zero, r2)
    }

    /// The language's shift amount: `x as i32` (saturating, NaN -> 0) kept only within 0..=63,
    /// which is exactly the x with 0 <= x < 64, truncated.
    fn shift_amount(&mut self, x: VReg) -> VReg {
        let t = self.trunc(x);
        let i = self.int();
        self.push(Inst::FloatToInt { dst: i, src: t });
        let zero_f = self.fconst(0.0);
        let sixty_four = self.fconst(64.0);
        let ge = self.fcmp(FCond::Oge, x, zero_f);
        let lt = self.fcmp(FCond::Olt, x, sixty_four);
        let inside = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: inside,
            a: ge,
            b: lt,
        });
        let zero = self.iconst(0);
        self.select_int(inside, i, zero)
    }

    /// `trunc(x)`, exactly, from the bits: an exponent below 0 leaves a signed zero, one of 52
    /// or more (integers, infinities, NaN) leaves `x`, anything between clears the fraction bits
    /// below the binary point.
    fn trunc(&mut self, x: VReg) -> VReg {
        let bits = self.int();
        self.push(Inst::FloatBits { dst: bits, src: x });
        let shifted = self.int();
        self.push(Inst::IntImm {
            op: IntOp::Sar,
            dst: shifted,
            a: bits,
            imm: 52,
        });
        let field = self.int();
        self.push(Inst::IntImm {
            op: IntOp::And,
            dst: field,
            a: shifted,
            imm: 0x7ff,
        });
        let e = self.int();
        self.push(Inst::IntImm {
            op: IntOp::Sub,
            dst: e,
            a: field,
            imm: 1023,
        });
        let small = self.int();
        self.push(Inst::ICmpImm {
            cond: ICond::Lt,
            dst: small,
            a: e,
            imm: 0,
        });
        let whole = self.int();
        self.push(Inst::ICmpImm {
            cond: ICond::Gt,
            dst: whole,
            a: e,
            imm: 51,
        });
        // The shift amount, kept within 0..=51 on every path (only one is selected).
        let zero = self.iconst(0);
        let fifty_one = self.iconst(51);
        let capped = self.select_int(whole, fifty_one, e);
        let amount = self.select_int(small, zero, capped);
        let fraction = self.iconst(0x000F_FFFF_FFFF_FFFF);
        let below = self.int();
        self.push(Inst::Int {
            op: IntOp::Sar,
            dst: below,
            a: fraction,
            b: amount,
        });
        let keep = self.int();
        self.push(Inst::Not {
            dst: keep,
            src: below,
        });
        let cleared = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: cleared,
            a: bits,
            b: keep,
        });
        let sign_mask = self.iconst(i64::MIN);
        let sign = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: sign,
            a: bits,
            b: sign_mask,
        });
        let r1 = self.select_int(whole, bits, cleared);
        let r2 = self.select_int(small, sign, r1);
        let t = self.float();
        self.push(Inst::BitsFloat { dst: t, src: r2 });
        t
    }
}

/// Constants defined by `IConst` in `func` (by register).
fn iconsts(func: &Func) -> std::collections::HashMap<u32, i64> {
    func.blocks
        .iter()
        .flat_map(|b| &b.insts)
        .filter_map(|i| match i {
            Inst::IConst { dst, value } => Some((dst.0, *value)),
            _ => None,
        })
        .collect()
}

/// An instruction native code hands to the host (`exec`) has no host to run it on a device:
/// name it.
fn check_host_calls(lengo: Lengo, func: &Func, function: &BytecodeFunc) -> Result<(), String> {
    let consts = iconsts(func);
    for inst in func.blocks.iter().flat_map(|b| &b.insts) {
        let Inst::Call { target, args, .. } = inst else {
            continue;
        };
        let pc = (*target == RtFn::Exec)
            .then(|| args.get(3).and_then(|p| consts.get(&p.0)))
            .flatten();
        let op = pc.and_then(|&pc| function.code.get(pc as usize));
        // Indexing runs natively; the host only raises its out-of-range error, which on a
        // device is the call failing.
        let error_path_only = matches!(op, Some(Opcode::ListGet { .. } | Opcode::ListSet { .. }));
        if lengo.runtime_symbol(*target).is_some() && (*target != RtFn::Exec || error_path_only) {
            continue;
        }
        let what = match op {
            Some(Opcode::Call(_)) => "mwito wa kazi inayopokea orodha (kifaa huita moja kwa moja \
                 kazi za Namba na Ukweli tu)"
                .to_string(),
            Some(Opcode::MakeNumList { .. } | Opcode::ListRepeat { .. }) => {
                "kuunda orodha (kifaa hakitengi kumbukumbu)".to_string()
            }
            Some(op) => format!("agizo `{}`", op.name()),
            None => format!("huduma ya mwenyeji `{target:?}`"),
        };
        return Err(format!("inahitaji mwenyeji kwa {what}"));
    }
    Ok(())
}

/// Lists reach the device as the caller's `double` arrays: a function whose analysis wants one
/// held narrower (as integers) cannot use them in place.
fn check_lists(func: &Func) -> Result<(), String> {
    let consts = iconsts(func);
    for inst in func.blocks.iter().flat_map(|b| &b.insts) {
        if let Inst::Call {
            target: RtFn::ListPtr,
            args,
            ..
        } = inst
        {
            let want = args.get(2).and_then(|w| consts.get(&w.0)).copied();
            if want != Some(Kind::F64.code() as i64) {
                return Err("inahifadhi orodha kama namba kamili, si kama Namba za kifaa".into());
            }
        }
    }
    Ok(())
}

/// The export of `f`: builds `f`'s frame on the stack from `hoja` (r0), calls its ordinary
/// entry, and stores the result at `matokeo` (r1). Returns the code and the offset of the `bl`
/// to link to the entry.
fn thunk(f: &BytecodeFunc) -> Result<(Vec<u8>, usize), String> {
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

fn store_pair(a: &mut Asm, lo: u8, hi: u8, off: u32) {
    if off <= 1020 {
        a.strd(lo, hi, SP, off);
    } else {
        a.add_any(12, SP, off);
        a.strd(lo, hi, 12, 0);
    }
}

/// C declarations of the exports.
fn header(lengo: Lengo, program: &BytecodeProgram, functions: &[usize], jina: &str) -> String {
    let guard: String = jina
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let (maelezo, lengo_jina, kifaa) = match lengo {
        Lengo::CortexM => (
            "Cortex-M (ARMv7E-M, fpv5-d16,\n * hard float)",
            "cortex-m",
            "libasili_kifaa.a",
        ),
        Lengo::Esp32 => ("ESP32 (Xtensa LX6,\n * ABI ya madirisha)", "esp32", ""),
    };
    let na_kifaa = if kifaa.is_empty() {
        String::new()
    } else {
        format!(" na {kifaa}")
    };
    let mut h = format!(
        "/* {jina}.h: kazi salama za Asili zilizojengwa kwa {maelezo} na `pata jenga --lengo \
         {lengo_jina}`.\n * Unganisha na {jina}-{lengo_jina}.o{na_kifaa}. Usihariri: faili hili \
         hutengenezwa upya\n * kila ujenzi.\n \
         *\n \
         * Kila kazi hurudisha 0 ikifaulu (matokeo yamewekwa), au namba isiyo sifuri ikishindwa\n \
         * (matokeo hayajawekwa): hapo programu iingie katika hali yake salama. Namba na Ukweli\n \
         * hupitishwa kama double (Ukweli: 0 au 1). */\n\n\
         #ifndef ASILI_{guard}_H\n#define ASILI_{guard}_H\n\n#include <stdint.h>\n\n\
         #ifndef ASILI_ORODHA\n#define ASILI_ORODHA\n\
         typedef struct {{\n    double *data;\n    uint32_t len;\n}} asili_orodha;\n#endif\n"
    );
    for &i in functions {
        let f = &program.functions[i];
        // The parameters' own names where the program kept its source (C-safe ones only).
        let names: Vec<String> = program
            .ast
            .as_ref()
            .and_then(|m| m.functions.iter().find(|g| g.name == f.name))
            .map(|g| g.params.iter().map(|p| p.name.to_string()).collect())
            .unwrap_or_default();
        let field = |k: usize| match names.get(k) {
            Some(n) if n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => n.clone(),
            _ => format!("hoja{k}"),
        };
        let hoja = if f.params.is_empty() {
            "const void *hoja".to_string()
        } else {
            h.push_str("\ntypedef struct {\n");
            for (k, p) in f.params.iter().enumerate() {
                let ty = if p.ty == Ty::List {
                    "asili_orodha"
                } else {
                    "double"
                };
                h.push_str(&format!("    {ty} {};\n", field(k)));
            }
            h.push_str(&format!("}} asili_{}_hoja;\n", f.name));
            format!("const asili_{}_hoja *hoja", f.name)
        };
        h.push_str(&format!(
            "int32_t asili_{}({hoja}, double *matokeo);\n",
            f.name
        ));
    }
    h.push_str(&format!("\n#endif /* ASILI_{guard}_H */\n"));
    h
}

/// A relocation in `.text`: at byte `at`, of type `kind`, against undefined symbol `symbol` —
/// or against `.text` itself with `addend` when `symbol` is `None` (Xtensa only: ARM's REL
/// relocations carry no addend).
struct ElfReloc {
    at: usize,
    symbol: Option<String>,
    kind: u32,
    addend: u32,
}

/// An ELF32 relocatable object holding `text`, with a global function symbol for each export
/// and its relocations: for ARM (EABI5, hard float) Thumb symbols and REL relocations, for
/// Xtensa RELA relocations.
fn elf_object(
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
