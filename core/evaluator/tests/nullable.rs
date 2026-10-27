//! Nullable bindings at runtime: a value stored in a `T?` is a `Chaguo`, whether it was declared
//! with a bare value, with `Hamna`, or by reassignment.

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

fn run(body: &str) -> String {
    match run_function(&compile(body), "jaribu", vec![]).expect("runs") {
        Value::Neno(s) => s.to_string(),
        other => panic!("expected Neno, got {other:?}"),
    }
}

#[test]
fn a_bare_value_in_a_nullable_binding_is_present() {
    let out = run(r#"
        kazi jaribu() -> Neno {
            weka x: Namba? = 4
            rejesha (x.angu(0)) kama Neno
        }
    "#);
    assert_eq!(out, "4");
}

#[test]
fn hamna_in_a_nullable_binding_is_empty() {
    let out = run(r#"
        kazi jaribu() -> Neno {
            weka x: Chaguo<Namba> = Hamna
            rejesha (x.ni_tupu()) kama Neno
        }
    "#);
    assert_eq!(out, "kweli");
}

#[test]
fn reassigning_a_bare_value_makes_a_nullable_binding_present() {
    let out = run(r#"
        kazi jaribu() -> Neno {
            weka x: Namba? = Hamna
            x = 7
            rejesha (x.angu(0)) kama Neno
        }
    "#);
    assert_eq!(out, "7");
}

#[test]
fn reassigning_hamna_makes_a_nullable_binding_empty() {
    let out = run(r#"
        kazi jaribu() -> Neno {
            weka x: Namba? = 4
            x = Hamna
            rejesha (x.ni_tupu()) kama Neno
        }
    "#);
    assert_eq!(out, "kweli");
}
