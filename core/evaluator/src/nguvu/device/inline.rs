//! Calls to functions taking a list are inlined into their callers: a device has no host frame
//! to pass the list through, and strict code has no recursion, so this ends.

use super::*;

/// Inline, into function `caller`, every call to a function taking a list, until none is left.
pub(super) fn inline_list_calls(
    program: &mut BytecodeProgram,
    caller: usize,
) -> Result<(), String> {
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
pub(super) fn inline_at(
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
