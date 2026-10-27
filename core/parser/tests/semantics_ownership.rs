//! Ownership and borrow rules, checked by the semantic analyzer: moves into a function, borrows
//! passed as parameters, and method and field lookup through a borrow.

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

/// A `kuu` body that may also declare helpers above it.
fn programu(helpers: &str, body: &str) -> String {
    format!("{helpers}\nkazi kuu(hoja: Orodha<Neno>) -> Tupu {{\n{body}\n}}\n")
}

const UMBO_S: &str = "umbo S { a: Neno }\n";

#[test]
fn passing_a_value_moves_it() {
    let src = programu(
        &format!("{UMBO_S}kazi f(s: S) -> Tupu {{ }}"),
        "    weka s = S { a: \"x\" }\n    f(s)\n    chapisha(s.a)",
    );
    assert_eq!(codes(&src), vec!["SEM040"]);
}

#[test]
fn passing_a_copy_value_does_not_move_it() {
    let src = programu(
        "kazi f(n: Namba) -> Tupu { }",
        "    weka n = 5\n    f(n)\n    chapisha((n kama Neno))",
    );
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn a_borrowed_argument_is_not_moved() {
    let src = programu(
        "kazi f(v: Rejeo<Orodha<Namba>>) -> Tupu { }",
        "    weka v = orodha(1, 2)\n    f(azima v)\n    chapisha((v.urefu()) kama Neno)",
    );
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn a_method_is_looked_up_through_a_borrow() {
    let src = programu(
        "",
        "    weka v = orodha(1, 2)\n    weka r = azima v\n    weka n = r.urefu()\n    chapisha((n) kama Neno)",
    );
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn a_field_is_looked_up_through_a_borrow() {
    let src = programu(
        UMBO_S,
        "    weka s = S { a: \"x\" }\n    weka r = azima s\n    chapisha(r.a)",
    );
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}
