//! Ubao (virtual board): `ubao_andika`, `ubao_soma`, `ubao_subiri`, `ubao_saa`. The state lives
//! in `crate::ubao`; these are the program-facing calls.

use std::collections::HashMap;

use super::BuiltinFn;
use crate::value::{self, EvalError, Value};

/// Argument `i` of `jina`: the pin's number.
fn pin(args: &[Value], i: usize, jina: &str) -> Result<f64, EvalError> {
    args.get(i)
        .and_then(value::as_f64)
        .ok_or_else(|| EvalError::TypeErr(format!("{jina}: namba ya pini inahitajika")))
}

/// Pin number `n` for `jina`: a whole, non-negative number.
fn pini(n: f64, jina: &str) -> Result<i64, EvalError> {
    if n.fract() != 0.0 || n < 0.0 {
        return Err(EvalError::TypeErr(format!(
            "{jina}: namba ya pini si sahihi ({n})"
        )));
    }
    Ok(n as i64)
}

fn hali(args: &[Value], i: usize, jina: &str) -> Result<bool, EvalError> {
    match args.get(i) {
        Some(Value::Ukweli(b)) => Ok(*b),
        _ => Err(EvalError::TypeErr(format!("{jina}: hali inahitaji Ukweli"))),
    }
}

/// `ubao_andika(pin, hali)`: the builtin's work, shared with the `PinWrite` instruction.
pub(crate) fn andika(pin: f64, hali: bool) -> Result<(), EvalError> {
    crate::ubao::andika(pini(pin, "ubao_andika")?, hali);
    Ok(())
}

/// `ubao_soma(pin)`, shared with the `PinRead` instruction.
pub(crate) fn soma(pin: f64) -> Result<bool, EvalError> {
    Ok(crate::ubao::soma(pini(pin, "ubao_soma")?))
}

/// `ubao_subiri(ms)` on a number, shared with the `BoardWait` instruction.
pub(crate) fn subiri(ms: f64) {
    crate::ubao::subiri(ms as i64);
}

/// `ubao_saa()`, shared with the `BoardClock` instruction.
pub(crate) fn saa() -> f64 {
    crate::ubao::saa() as f64
}

pub(crate) fn register(m: &mut HashMap<String, BuiltinFn>) {
    m.insert(
        "ubao_andika".to_string(),
        Box::new(|args: &[Value]| {
            let p = pin(args, 0, "ubao_andika")?;
            let h = hali(args, 1, "ubao_andika")?;
            andika(p, h)?;
            Ok(Value::Tupu)
        }),
    );
    m.insert(
        "ubao_soma".to_string(),
        Box::new(|args: &[Value]| {
            let p = pin(args, 0, "ubao_soma")?;
            Ok(Value::Ukweli(soma(p)?))
        }),
    );
    m.insert(
        "ubao_subiri".to_string(),
        Box::new(|args: &[Value]| {
            let ms = args.first().and_then(value::as_f64).ok_or_else(|| {
                EvalError::TypeErr("ubao_subiri: idadi ya milisekunde inahitajika".into())
            })?;
            subiri(ms);
            Ok(Value::Tupu)
        }),
    );
    m.insert(
        "ubao_saa".to_string(),
        Box::new(|_args: &[Value]| Ok(Value::Namba(saa()))),
    );
}
