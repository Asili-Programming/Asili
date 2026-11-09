//! The device targets: a program's `#[salama]` and `#[wazi]` functions compiled to machine code
//! as an ELF relocatable object a firmware links, plus a C header declaring them
//! (`pata jenga --lengo cortex-m` / `--lengo esp32`). Cortex-M code comes from
//! [`super::codegen_t32`] (Thumb-2), ESP32 code from [`super::codegen_xt`] (Xtensa LX6).
//!
//! Each export `f` has the C interface
//! `int32_t asili_f(const asili_f_hoja *hoja, double *matokeo)`: `hoja` holds its arguments in
//! order (a `Namba` or `Ukweli` as a `double`, an `Orodha<Namba>` as `asili_orodha`, a pointer
//! to the caller's `double` array and its length, eight bytes each); the result is stored at
//! `matokeo`, and the return value is 0, or non-zero when the call failed (an index out of range,
//! say) — the firmware's cue to enter its safe state. On Cortex-M the export is a small thunk that
//! builds the function's register frame and calls its native code, which calls the device runtime
//! (`asili_kifaa_*`); on the ESP32 the export is the function itself where it can be
//! ([`interface::export_interface`]), the runtime is compiled in ([`esp32`]), and a `#[wazi] kuu`
//! is the firmware's entry point. Functions call each other directly.
//!
//! Only what a device can run without a host is accepted: numbers, truth values and the caller's
//! numeric lists, with no allocation — anything else is a build error. Everything up to the IR
//! (checks, inlining, lowering, optimization) is shared by the targets.
//!
//! - [`object`]: each target's object — its units, thunks and stack report.
//! - [`interface`]: the C interface — exports built into the IR, thunk descriptions, the header.
//! - [`inline`]: inlining calls to functions taking lists (a device passes no lists by frame).
//! - [`esp32`]: the ESP32's runtime in IR — list access, conversions, GPIO and the clock.
//! - [`elf`]: the ELF writer.

use super::codegen_t32;
use super::codegen_xt;
use super::ir::{BlockData, Class, FCond, FloatOp, Func, ICond, Inst, IntOp, RtFn, Term, VReg};
use super::t32::{self, Asm, Cond, SP};
use super::{lower, opt, verify};
use crate::bytecode::{BytecodeFunc, BytecodeProgram, Opcode, Operand, Ty};
use crate::numlist::Kind;

mod elf;
mod esp32;
mod inline;
mod interface;
mod object;

use elf::*;
use esp32::*;
use inline::*;
use interface::*;
use object::*;

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
            Lengo::Esp32 => match target {
                // The board's pins (inlined) and clock (a helper).
                RtFn::PinWrite | RtFn::PinRead | RtFn::BoardWait | RtFn::BoardClock => Some(""),
                _ => codegen_t32::runtime_symbol(target)
                    .map(|_| codegen_xt::runtime_symbol(target).unwrap_or("")),
            },
        }
    }
}

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
    build_firmware(lengo, program, functions, None, jina)
}

/// [`build_for`], with function `kuu` (the program's `#[wazi] kuu`) also built as the
/// firmware's entry point, `void app_main(void)`, which ESP-IDF calls at start-up (ESP32 only).
pub fn build_firmware(
    lengo: Lengo,
    program: &BytecodeProgram,
    functions: &[usize],
    kuu: Option<usize>,
    jina: &str,
) -> Result<Kifaa, String> {
    for &i in functions {
        let f = &program.functions[i];
        signature(f).map_err(|e| kosa(lengo, f, &e))?;
    }
    if kuu.is_some() && lengo != Lengo::Esp32 {
        return Err(format!(
            "kuu ya #[wazi] hujengwa kama mwanzo wa programu kwa ESP32 pekee, si {}",
            lengo.jina()
        ));
    }
    // A strict function calling one that takes a list would go through the host: on a device
    // the callee's body is inlined instead (strict code has no recursion, so this ends).
    let mut inlined = program.clone();
    for &i in functions.iter().chain(&kuu) {
        inline_list_calls(&mut inlined, i).map_err(|e| kosa(lengo, &program.functions[i], &e))?;
    }
    crate::bytecode_verify::verify(&inlined).map_err(|e| format!("kosa la ndani: {e}"))?;
    let program = &inlined;
    let (object, stack) = match lengo {
        Lengo::CortexM => (object_t32(program, functions)?, Vec::new()),
        Lengo::Esp32 => object_xt(program, functions, kuu)?,
    };
    let mut header = header(lengo, program, functions, jina);
    let mut exports: Vec<String> = functions
        .iter()
        .map(|&i| program.functions[i].name.clone())
        .collect();
    if kuu.is_some() {
        if let Some(end) = header.rfind("\n#endif") {
            header.insert_str(
                end,
                "\n/* kuu ya programu imejengwa kama app_main: ESP-IDF huiita yenyewe. */\n",
            );
        }
        exports.push("kuu (app_main)".into());
    }
    Ok(Kifaa {
        object,
        stack,
        header,
        exports,
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
            device: true,
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

fn kosa(lengo: Lengo, f: &BytecodeFunc, e: &str) -> String {
    format!(
        "kazi '{}' haiwezi kujengwa kwa {}: {e}",
        f.name,
        lengo.jina()
    )
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

// ----- ESP32 GPIO and clock (registers from ESP-IDF's `soc/esp32` headers) -----------------

/// Status of a call that cannot be made on a device (`native::STATUS_FAIL << 32`).
const STATUS_FAIL: i64 = 2 << 32;

/// What `exec` returns on a device: the call fails (`native::STATUS_FAIL`); it is only reached
/// on an error path (see [`check_host_calls`]).
const EXEC_FAIL: i64 = 2;

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
            None if matches!(
                target,
                RtFn::PinWrite | RtFn::PinRead | RtFn::BoardWait | RtFn::BoardClock
            ) =>
            {
                "pini na saa za ubao (kwa sasa ESP32 pekee inaziendesha)".to_string()
            }
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
