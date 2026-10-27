//! Fixed-width integers at runtime: arithmetic wraps at the width, division truncates, and the
//! fallible casts give `Kuna` or `Hamna`.

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

/// The text `jaribu()` returns for a body of statements, which ends in `rejesha`.
fn show(body: &str) -> String {
    let src = format!("kazi jaribu() -> Neno {{\n{body}\n}}\n");
    match run_function(&compile(&src), "jaribu", vec![]).expect("runs") {
        Value::Neno(s) => s.to_string(),
        other => panic!("expected Neno, got {other:?}"),
    }
}

#[test]
fn addition_wraps_at_the_width() {
    let out = show(
        r#"
        weka a: Biti8 = 127
        weka b = a + 1
        rejesha (b) kama Neno"#,
    );
    assert_eq!(out, "-128");
}

#[test]
fn subtraction_wraps_for_an_unsigned_width() {
    let out = show(
        r#"
        weka a: uBiti8 = 0
        weka b = a - 1
        rejesha (b) kama Neno"#,
    );
    assert_eq!(out, "255");
}

#[test]
fn a_reassigned_value_wraps_too() {
    let out = show(
        r#"
        weka a: Biti8 = 1
        a = 127
        a = a + 1
        rejesha (a) kama Neno"#,
    );
    assert_eq!(out, "-128");
}

#[test]
fn division_truncates_toward_zero() {
    let out = show(
        r#"
        weka a: Biti32 = -7
        weka b = a / 2
        rejesha (b) kama Neno"#,
    );
    assert_eq!(out, "-3");
}

#[test]
fn remainder_takes_the_sign_of_the_dividend() {
    let out = show(
        r#"
        weka a: Biti32 = -7
        weka b = a % 3
        rejesha (b) kama Neno"#,
    );
    assert_eq!(out, "-1");
}

#[test]
fn dividing_by_zero_halts() {
    let src = r#"
        kazi jaribu() -> Neno {
            weka a: Biti32 = 5
            weka z: Biti32 = 0
            rejesha (a / z) kama Neno
        }
    "#;
    assert!(run_function(&compile(src), "jaribu", vec![]).is_err());
}

#[test]
fn a_fallible_cast_in_range_is_present() {
    let out = show("rejesha (3.9 kama Biti32) kama Neno");
    assert_eq!(out, "Kuna(3)");
}

#[test]
fn a_fallible_cast_out_of_range_is_empty() {
    assert_eq!(show("rejesha (300 kama uBiti8) kama Neno"), "Hamna");
    assert_eq!(show("rejesha (-1 kama uBiti8) kama Neno"), "Hamna");
}

#[test]
fn sixty_four_bit_values_stay_exact() {
    let out = show(r#"rejesha ("18446744073709551615" kama uBiti64) kama Neno"#);
    assert_eq!(out, "Kuna(18446744073709551615)");
}

#[test]
fn a_comparison_gives_a_bool() {
    let out = show(
        r#"
        weka a: Biti8 = 5
        rejesha (a < 6) kama Neno"#,
    );
    assert_eq!(out, "kweli");
}
