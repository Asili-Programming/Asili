//! `.clona()`: a copy of any value. A `umbo` gets it for free, unless its program defines its own
//! `clona`; a `Chaguo` and a `Tokeo` copy the same way as a list does.

use asili_evaluator::{run_function, Value};
use asili_lexer::tokenize;
use asili_parser::{extern_env_from_imports, parse_tokens, semantic_check_with_env, Module};

fn compile(body: &str) -> Module {
    let toks = tokenize(body).expect("tokenize");
    let module = parse_tokens(&toks).unwrap_or_else(|e| panic!("parse: {e:?}"));
    let (fns, consts) = extern_env_from_imports(&module);
    if let Err(e) = semantic_check_with_env(&module, false, fns, consts) {
        panic!("semantic check: {e:?}");
    }
    module
}

fn run(body: &str) -> Value {
    run_function(&compile(body), "jaribu", vec![]).expect("runs")
}

fn text(v: Value) -> String {
    match v {
        Value::Neno(s) => s.to_string(),
        other => panic!("expected Neno, got {other:?}"),
    }
}

#[test]
fn a_struct_is_copied_by_clona() {
    let out = run(r#"
        umbo S { a: Neno }
        kazi jaribu() -> Neno {
            weka s = S { a: "x" }
            weka t = s.clona()
            rejesha t.a
        }
    "#);
    assert_eq!(text(out), "x");
}

#[test]
fn a_program_defined_clona_takes_precedence() {
    let out = run(r#"
        umbo S { a: Neno }
        shughuli ya S {
            kazi clona(self: S) -> Neno { rejesha "yangu" }
        }
        kazi jaribu() -> Neno {
            weka s = S { a: "x" }
            rejesha s.clona()
        }
    "#);
    assert_eq!(text(out), "yangu");
}

#[test]
fn a_chaguo_is_copied_by_clona() {
    let out = run(r#"
        kazi jaribu() -> Neno {
            weka c = Chaguo::Kuna(3)
            weka d = c.clona()
            rejesha (d.angu(0)) kama Neno
        }
    "#);
    assert_eq!(text(out), "3");
}
