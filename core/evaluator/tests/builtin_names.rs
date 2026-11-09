//! A program's builtin calls are indices into the sorted list of every builtin's name, so the
//! list must not depend on how the runtime is built: a runner without networking (`mtandao`)
//! registers stubs under the same names. This checks the list against `golden/builtin_names.txt`
//! in whatever configuration it runs — CI runs it with and without default features. Record with
//! `ASILI_GOLDEN=write`.

#[test]
fn builtin_names_are_the_same_in_every_build() {
    let names = asili_evaluator::builtins::builtin_names().join("\n") + "\n";
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/builtin_names.txt"
    );
    if std::env::var_os("ASILI_GOLDEN").is_some_and(|v| v == "write") {
        std::fs::write(path, &names).expect("write golden");
    }
    let golden = std::fs::read_to_string(path).expect("golden/builtin_names.txt");
    assert_eq!(
        names, golden,
        "builtin names changed (or differ in this build)"
    );
}

#[test]
fn every_network_builtin_exists() {
    let names = asili_evaluator::builtins::builtin_names();
    for n in asili_evaluator::builtins::NETWORK_BUILTINS {
        assert!(names.iter().any(|m| m == n), "{n} is not a builtin");
    }
}

#[test]
fn needs_network_sees_the_programs_that_use_it() {
    let program = |source: &str| {
        let tokens = asili_lexer::tokenize(source).expect("tokenize");
        let module = asili_parser::parse_tokens(&tokens).expect("parse");
        asili_evaluator::compile_module_explained(&module).expect("bytecode")
    };
    let plain = program("kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n    chapisha(\"habari\")\n}\n");
    assert!(!asili_evaluator::builtins::needs_network(&plain));
    let fetch = program(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n    weka j = http_ombi(\"GET\", \"http://example.com\")\n    chapisha(\"x\")\n}\n",
    );
    assert!(asili_evaluator::builtins::needs_network(&fetch));
    let resolve = program(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n    weka a = tafuta_anwani(\"localhost\")\n}\n",
    );
    assert!(asili_evaluator::builtins::needs_network(&resolve));
}
