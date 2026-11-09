//! Virtual board (`ubao`): a program drives pins and reads a scripted scenario, and the board
//! records every event. The same program with the same scenario gives the same events and result
//! on native code and on the reference path, and pin calls outside a `#[wazi]` grant are errors.

use asili_evaluator::ubao::{Ingizo, Tukio};
use asili_evaluator::{
    compile_module_explained, run_bytecode_function_on, run_function, ubao, Value,
};
use asili_lexer::tokenize;
use asili_parser::{extern_env_from_imports, parse_tokens, semantic_check_with_env, Module};

const PROGRAM: &str = r#"
leta ubao

#[wazi]
kazi prova() -> Namba {
    ubao_andika(3, kweli)
    ubao_subiri(10)
    weka a = ubao_soma(3)
    ubao_andika(4, a)
    rejesha ubao_saa()
}
"#;

fn scenario() -> Vec<Ingizo> {
    vec![
        Ingizo {
            saa: 0,
            pini: 3,
            hali: false,
        },
        Ingizo {
            saa: 5,
            pini: 3,
            hali: true,
        },
    ]
}

fn parse(src: &str) -> Module {
    parse_tokens(&tokenize(src).expect("tokenize")).expect("parse")
}

fn expected_events() -> Vec<Tukio> {
    vec![
        Tukio {
            hatua: 0,
            aina: "andika",
            pini: 3,
            hali: true,
            saa: 0,
        },
        Tukio {
            hatua: 1,
            aina: "subiri",
            pini: -1,
            hali: false,
            saa: 10,
        },
        Tukio {
            hatua: 2,
            aina: "soma",
            pini: 3,
            hali: true,
            saa: 10,
        },
        Tukio {
            hatua: 3,
            aina: "andika",
            pini: 4,
            hali: true,
            saa: 10,
        },
    ]
}

#[test]
fn program_drives_pins_and_reads_the_scenario() {
    let module = parse(PROGRAM);
    ubao::anza(scenario());
    let got = run_function(&module, "prova", vec![]).expect("run");
    assert_eq!(got, Value::Namba(10.0));
    assert_eq!(ubao::tukio(), expected_events());
}

#[test]
fn native_code_and_the_reference_path_agree() {
    if !asili_evaluator::nguvu::supported() {
        return;
    }
    let module = parse(PROGRAM);
    let program = compile_module_explained(&module).expect("bytecode lowering");
    let own = asili_evaluator::nguvu::compile(&program).expect("nguvu compile");

    ubao::anza(scenario());
    let native = run_bytecode_function_on(&own, &program, "prova", vec![]).expect("native");
    let native_events = ubao::tukio();

    ubao::anza(scenario());
    let reference = run_function(&module, "prova", vec![]).expect("reference");
    let reference_events = ubao::tukio();

    assert_eq!(native, reference);
    assert_eq!(native_events, reference_events);
    assert_eq!(native_events, expected_events());
}

#[test]
fn a_pin_read_before_any_change_is_low() {
    let module = parse("leta ubao\n#[wazi]\nkazi prova() -> Ukweli { rejesha ubao_soma(9) }");
    ubao::anza(vec![Ingizo {
        saa: 0,
        pini: 3,
        hali: true,
    }]);
    assert_eq!(
        run_function(&module, "prova", vec![]).expect("run"),
        Value::Ukweli(false)
    );
}

#[test]
fn pin_calls_need_a_wazi_grant() {
    let src = "leta ubao\nkazi prova() -> Tupu { ubao_andika(1, kweli) }";
    let module = parse(src);
    let (functions, constants) = extern_env_from_imports(&module);
    let errs = semantic_check_with_env(&module, false, functions, constants).unwrap_err();
    let text = format!("{errs:?}");
    assert!(text.contains("SEM121"), "{text}");
}

#[test]
fn a_negative_pin_is_an_error() {
    let module = parse("leta ubao\n#[wazi]\nkazi prova() -> Tupu { ubao_andika(-1, kweli) }");
    ubao::anza(vec![]);
    assert!(run_function(&module, "prova", vec![]).is_err());
}

#[test]
fn typed_pin_calls_are_instructions_of_their_own() {
    // A number and a truth value go straight to `PinWrite`/`PinRead`/`BoardWait` (no boxing),
    // which native code on a device can run without a host; anything else stays a builtin call.
    let module = parse(
        "leta ubao\n#[wazi]\nkazi prova(p: Namba) -> Ukweli {\n    ubao_andika(p, kweli)\n    ubao_subiri(3)\n    rejesha ubao_soma(p)\n}",
    );
    let program = compile_module_explained(&module).expect("compile");
    let f = program
        .functions
        .iter()
        .find(|f| f.name == "prova")
        .expect("prova");
    let code = format!("{:?}", f.code);
    for op in ["PinWrite", "BoardWait", "PinRead"] {
        assert!(code.contains(op), "{op} missing: {code}");
    }
    assert!(!code.contains("CallBuiltin"), "{code}");
    ubao::anza(vec![]);
    assert_eq!(
        run_function(&module, "prova", vec![Value::Namba(5.0)]).expect("run"),
        Value::Ukweli(false)
    );
    let events: Vec<(&str, i64, bool, i64)> = ubao::tukio()
        .iter()
        .map(|t| (t.aina, t.pini, t.hali, t.saa))
        .collect();
    assert_eq!(
        events,
        vec![
            ("andika", 5, true, 0),
            ("subiri", -1, false, 3),
            ("soma", 5, false, 3)
        ]
    );
}

#[test]
fn a_fractional_pin_is_an_error_on_the_instruction_path() {
    let module = parse("leta ubao\n#[wazi]\nkazi prova() -> Tupu { ubao_andika(1.5, kweli) }");
    ubao::anza(vec![]);
    let err = format!("{:?}", run_function(&module, "prova", vec![]).unwrap_err());
    assert!(err.contains("namba ya pini si sahihi"), "{err}");
}
