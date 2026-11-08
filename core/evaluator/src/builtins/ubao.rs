//! Ubao (virtual board): `ubao_andika`, `ubao_soma`, `ubao_subiri`, `ubao_saa`. The state lives
//! in `crate::ubao`; these are the program-facing calls.

use std::collections::HashMap;

use super::BuiltinFn;
use crate::value::{self, EvalError, Value};

fn pin(args: &[Value], i: usize, jina: &str) -> Result<i64, EvalError> {
    let n = args
        .get(i)
        .and_then(value::as_f64)
        .ok_or_else(|| EvalError::TypeErr(format!("{jina}: namba ya pini inahitajika")))?;
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

pub(crate) fn register(m: &mut HashMap<String, BuiltinFn>) {
    m.insert(
        "ubao_andika".to_string(),
        Box::new(|args: &[Value]| {
            let p = pin(args, 0, "ubao_andika")?;
            let h = hali(args, 1, "ubao_andika")?;
            crate::ubao::andika(p, h);
            Ok(Value::Tupu)
        }),
    );
    m.insert(
        "ubao_soma".to_string(),
        Box::new(|args: &[Value]| {
            let p = pin(args, 0, "ubao_soma")?;
            Ok(Value::Ukweli(crate::ubao::soma(p)))
        }),
    );
    m.insert(
        "ubao_subiri".to_string(),
        Box::new(|args: &[Value]| {
            let ms = args.first().and_then(value::as_f64).ok_or_else(|| {
                EvalError::TypeErr("ubao_subiri: idadi ya milisekunde inahitajika".into())
            })?;
            crate::ubao::subiri(ms as i64);
            Ok(Value::Tupu)
        }),
    );
    m.insert(
        "ubao_saa".to_string(),
        Box::new(|_args: &[Value]| Ok(Value::Namba(crate::ubao::saa() as f64))),
    );
}
