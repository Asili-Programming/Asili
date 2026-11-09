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
    let golden = std::fs::read_to_string(path)
        .expect("golden/builtin_names.txt")
        .replace("\r\n", "\n");
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
fn needed_names_the_features_a_program_uses() {
    use asili_evaluator::uwezo::needed;
    let program = |body: &str| {
        let source = format!("kazi kuu(hoja: Orodha<Neno>) -> Tupu {{\n{body}\n}}\n");
        let tokens = asili_lexer::tokenize(&source).expect("tokenize");
        let module = asili_parser::parse_tokens(&tokens).expect("parse");
        asili_evaluator::compile_module_explained(&module).expect("bytecode")
    };
    assert!(needed(&program("    chapisha(\"habari\")")).is_empty());
    assert_eq!(
        needed(&program(
            "    weka j = http_ombi(\"GET\", \"http://example.com\")"
        )),
        ["mtandao"]
    );
    assert_eq!(
        needed(&program("    weka a = tafuta_anwani(\"localhost\")")),
        ["mtandao"]
    );
    assert_eq!(
        needed(&program("    weka r = ruwaza_inalingana(\"a+\", \"aaa\")")),
        ["ruwaza"]
    );
    assert_eq!(
        needed(&program(
            "    weka r = ruwaza_tafuta(\"a\", \"a\")\n    weka j = http_ombi(\"GET\", \"x\")"
        )),
        ["mtandao", "ruwaza"]
    );
}

#[test]
fn a_runner_says_what_it_can_run() {
    use asili_evaluator::uwezo::{ya_tenda, UWEZO, UWEZO_ALAMA};
    // The marker as built into this crate, amid other bytes (and a stray, unterminated prefix).
    let mut file = b"\x7fELF...\0ASILI-UWEZO:\x01\x02junk".to_vec();
    file.extend_from_slice(UWEZO_ALAMA);
    file.extend_from_slice(b"...more");
    assert_eq!(ya_tenda(&file).expect("marker"), UWEZO);
    assert_eq!(
        ya_tenda(b"\0ASILI-UWEZO:;").expect("marker"),
        Vec::<String>::new()
    );
    assert_eq!(ya_tenda(b"an old runner"), None);
}
