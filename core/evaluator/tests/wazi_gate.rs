//! `#[wazi]`: the grant for operations that reach past the language (`syscall`). Only a function
//! marked `#[wazi]` may call `syscall`, or call another `#[wazi]` function (SEM121); a `#[wazi]`
//! function may not also be `#[salama]`, since a syscall has no time bound (SEM122).

use asili_lexer::tokenize;
use asili_parser::{extern_env_from_imports, parse_tokens, semantic_check_with_env, Module};

fn compile(body: &str) -> Result<Module, String> {
    let src = format!("leta syscall\n{body}");
    let tokens = tokenize(&src).map_err(|e| format!("{e:?}"))?;
    let module = parse_tokens(&tokens).map_err(|e| format!("{e:?}"))?;
    let (functions, constants) = extern_env_from_imports(&module);
    semantic_check_with_env(&module, true, functions, constants)
        .map(|_| module)
        .map_err(|errs| {
            errs.iter()
                .map(|d| format!("{d:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

#[test]
fn syscall_needs_a_wazi_grant() {
    let err = compile("kazi kuu(hoja: Orodha<Neno>) -> Tupu { weka r = syscall(1, 0, 0, 0)\n }")
        .unwrap_err();
    assert!(err.contains("SEM121"), "{err}");
}

#[test]
fn syscall_inside_a_wazi_function_is_allowed() {
    compile(
        "#[wazi]\nkazi raw() -> Namba { weka r = syscall(1, 0, 0, 0)\n rejesha 0 }\n\
         kazi kuu(hoja: Orodha<Neno>) -> Tupu { }",
    )
    .expect("a #[wazi] function may call syscall");
}

#[test]
fn wazi_function_needs_a_grant_at_its_call_site() {
    let err = compile(
        "#[wazi]\nkazi raw() -> Namba { rejesha 0 }\n\
         kazi kuu(hoja: Orodha<Neno>) -> Tupu { weka r = raw()\n }",
    )
    .unwrap_err();
    assert!(err.contains("SEM121"), "{err}");
}

#[test]
fn wazi_may_call_wazi() {
    compile(
        "#[wazi]\nkazi inner() -> Namba { rejesha 0 }\n\
         #[wazi]\nkazi outer() -> Namba { weka r = inner()\n rejesha r }\n\
         kazi kuu(hoja: Orodha<Neno>) -> Tupu { }",
    )
    .expect("a grant may call another grant");
}

#[test]
fn wazi_and_salama_do_not_combine() {
    let err = compile(
        "#[wazi]\n#[salama]\nkazi raw(n: Namba) -> Namba { rejesha n }\n\
         kazi kuu(hoja: Orodha<Neno>) -> Tupu { }",
    )
    .unwrap_err();
    assert!(err.contains("SEM122"), "{err}");
}

#[test]
fn wazi_is_not_an_item_attribute() {
    let err = compile("#[wazi]\numbo Nukta { x: Namba }\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { }")
        .unwrap_err();
    assert!(err.contains("SEM008"), "{err}");
}
