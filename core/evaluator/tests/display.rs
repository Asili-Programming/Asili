//! `x kama Neno`: how a value reads to a program's user. Scalars read as before; `Chaguo`,
//! `Tokeo`, lists and structs read in Swahili, not as Rust debug output.

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

/// The text `jaribu()` returns for `body`, which must be a `Neno`.
fn show(body: &str) -> String {
    let src = format!("kazi jaribu() -> Neno {{\n{body}\n}}\n");
    match run_function(&compile(&src), "jaribu", vec![]).expect("runs") {
        Value::Neno(s) => s.to_string(),
        other => panic!("expected Neno, got {other:?}"),
    }
}

#[test]
fn scalars_read_as_they_always_have() {
    assert_eq!(show("rejesha (2.5) kama Neno"), "2.5");
    assert_eq!(show("rejesha (kweli) kama Neno"), "kweli");
    assert_eq!(show("rejesha (si_kweli) kama Neno"), "si_kweli");
    assert_eq!(show("rejesha \"habari\""), "habari");
}

#[test]
fn a_chaguo_reads_as_kuna_or_hamna() {
    assert_eq!(show("rejesha (Chaguo::Kuna(3)) kama Neno"), "Kuna(3)");
    assert_eq!(show("rejesha (Chaguo::Hamna) kama Neno"), "Hamna");
}

#[test]
fn a_tokeo_reads_as_sawa_or_kosa() {
    assert_eq!(show("rejesha (Tokeo::Sawa(2)) kama Neno"), "Sawa(2)");
    assert_eq!(show("rejesha (Tokeo::Kosa(\"bad\")) kama Neno"), "Kosa(\"bad\")");
}

#[test]
fn a_list_reads_with_quoted_text() {
    assert_eq!(show("rejesha (orodha(\"a\", \"b\")) kama Neno"), "[\"a\", \"b\"]");
}

#[test]
fn a_struct_reads_with_its_fields() {
    let src = r#"
        umbo S { a: Neno, b: Namba }
        kazi jaribu() -> Neno {
            rejesha (S { a: "x", b: 2 }) kama Neno
        }
    "#;
    match run_function(&compile(src), "jaribu", vec![]).expect("runs") {
        Value::Neno(s) => assert_eq!(&*s, "S { a: \"x\", b: 2 }"),
        other => panic!("expected Neno, got {other:?}"),
    }
}
