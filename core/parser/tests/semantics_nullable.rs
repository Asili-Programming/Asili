//! Nullable bindings (`T?`, `Chaguo<T>`): which declarations and reassignments check.

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
fn a_nullable_binding_takes_hamna() {
    assert!(codes(&kuu("    weka x: Namba? = Hamna")).is_empty());
    assert!(codes(&kuu("    weka x: Chaguo<Namba> = Hamna")).is_empty());
}

#[test]
fn a_nullable_binding_takes_a_bare_value() {
    assert!(codes(&kuu("    weka x: Namba? = 4")).is_empty());
}

#[test]
fn a_nullable_binding_still_rejects_the_wrong_type() {
    assert_eq!(codes(&kuu("    weka x: Namba? = \"a\"")), vec!["SEM010"]);
}

#[test]
fn reassigning_a_nullable_binding_takes_hamna_and_a_bare_value() {
    let src = kuu("    weka x: Namba? = Hamna\n    x = 7\n    x = Hamna");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn reassigning_a_nullable_binding_still_rejects_the_wrong_type() {
    let src = kuu("    weka x: Namba? = 1\n    x = \"a\"");
    assert_eq!(codes(&src), vec!["SEM013"]);
}
