//! The C interface of the exports: an ESP32 export built into its direct entry's IR, what a
//! thunk needs to know, and the header declaring them.

use super::*;

/// Build an export's C interface — `int32_t asili_f(const hoja *h, double *matokeo)` — into the
/// IR of `f`'s direct entry, so the export is that function with no thunk in between: its
/// registers' reads become reads of the arguments in `hoja` (eight bytes each, list references
/// included, in parameter order), its number is stored at `matokeo` with 0 returned, and a
/// failing status returns its kind (non-zero). `false` (and `func` to be dropped) when the
/// entry uses its register buffer otherwise, or the host fields.
pub(super) fn export_interface(func: &mut Func, f: &BytecodeFunc) -> bool {
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
    let buffer = |v: &VReg| *v == crate::nguvu::ir::NUMS || *v == crate::nguvu::ir::HOST;
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            if let Inst::Load { base, offset, .. } = inst {
                if *base == crate::nguvu::ir::NUMS {
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
                    base: crate::nguvu::ir::HOST, // `matokeo`
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
    func.args = crate::nguvu::ir::EXPORT_ARGS;
    true
}

/// What the ESP32 export thunk of `f` needs (see [`codegen_xt::thunk`]).
pub(super) fn thunk_spec(f: &BytecodeFunc, direct: bool) -> codegen_xt::ThunkSpec {
    codegen_xt::ThunkSpec {
        direct,
        entry_point: false,
        start_clock: None,
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

/// C declarations of the exports.
pub(super) fn header(
    lengo: Lengo,
    program: &BytecodeProgram,
    functions: &[usize],
    jina: &str,
) -> String {
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
