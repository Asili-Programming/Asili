//! Fixed-width integers (`Biti8`…`uBiti64`): literal ranges, widths in arithmetic, and the
//! types a binding or an operator gives.

use asili_lexer::tokenize;
use asili_parser::{parse_tokens, semantic_check};

/// The diagnostic codes `src` produces, in order. An empty list means the program checks.
fn codes(src: &str) -> Vec<&'static str> {
    let tokens = tokenize(src).expect("tokenize");
    let module = parse_tokens(&tokens).expect("parse");
    semantic_check(&module)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.code)
        .collect()
}

/// A `kuu` whose body is `body`.
fn kuu(body: &str) -> String {
    format!("kazi kuu(hoja: Orodha<Neno>) -> Tupu {{\n{body}\n}}\n")
}

#[test]
fn a_literal_in_range_binds_to_its_width() {
    assert!(codes(&kuu("    weka a: Biti8 = 127")).is_empty());
    assert!(codes(&kuu("    weka a: Biti8 = -128")).is_empty());
    assert!(codes(&kuu("    weka a: uBiti8 = 255")).is_empty());
}

#[test]
fn a_literal_out_of_range_is_an_error() {
    assert_eq!(codes(&kuu("    weka a: Biti8 = 128")), vec!["SEM108"]);
    assert_eq!(codes(&kuu("    weka a: uBiti8 = -1")), vec!["SEM108"]);
    assert_eq!(codes(&kuu("    weka a: Biti8 = 3.5")), vec!["SEM108"]);
}

#[test]
fn reassigning_a_literal_is_range_checked_too() {
    assert_eq!(
        codes(&kuu("    weka a: Biti8 = 5\n    a = 300")),
        vec!["SEM108"]
    );
    assert!(codes(&kuu("    weka a: Biti8 = 5\n    a = 6")).is_empty());
}

#[test]
fn a_non_literal_namba_does_not_bind_to_a_width() {
    assert_eq!(codes(&kuu("    weka a: Biti8 = \"x\"")), vec!["SEM010"]);
    assert_eq!(
        codes(&kuu("    weka n = 5\n    weka a: Biti8 = n")),
        vec!["SEM010"]
    );
}

#[test]
fn two_widths_of_one_name_add_and_a_literal_takes_the_width() {
    let src = kuu("    weka a: Biti8 = 5\n    weka b: Biti8 = 6\n    weka c: Biti8 = a + b\n    weka d: Biti8 = a + 1");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn a_comparison_of_fixed_widths_is_a_bool() {
    let src = kuu("    weka a: Biti8 = 5\n    weka c: Ukweli = a < 6");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn mixing_widths_is_an_error() {
    let src = kuu("    weka a: Biti8 = 5\n    weka b: Biti32 = 6\n    weka c = a + b");
    assert_eq!(codes(&src), vec!["SEM033"]);
}

#[test]
fn a_non_literal_namba_with_a_fixed_width_is_an_error() {
    let src = kuu("    weka a: Biti8 = 5\n    weka n = 2\n    weka c = a + n");
    assert_eq!(codes(&src), vec!["SEM033"]);
}

#[test]
fn an_out_of_range_literal_in_an_operation_is_one_error() {
    let src = kuu("    weka a: Biti8 = 5\n    weka c = a + 300");
    assert_eq!(codes(&src), vec!["SEM108"]);
}

#[test]
fn a_width_does_not_bind_to_another_width_without_a_cast() {
    let src = kuu("    weka a: Biti8 = 5\n    weka c: Biti16 = a");
    assert_eq!(codes(&src), vec!["SEM010"]);
}
