//! Asili Wasm driver: runs Asili source in the browser (`wasm-browser`). Programs are lowered to
//! bytecode and run as native code — a wasm module of their own the page instantiates beside
//! this one (`asili_evaluator::nguvu::wasm`). I/O builtins (chapisha, majira, …) route through
//! `asili_evaluator`'s platform shim (console.log / console.error in the browser).

use asili_evaluator::{compile_module_explained, run_bytecode};
use asili_lexer::tokenize;
use asili_parser::{
    extern_env_from_imports, merge_modules, parse_tokens, semantic_check_with_env, ImportPath, Module,
};
use std::collections::HashMap;

/// Run Asili source: tokenize, parse, run `kuu` with empty args. Single-module only — any `leta`
/// import is left unresolved (its exports just won't be found). For multi-module programs use
/// `run_bundle`.
pub fn run_source(source: &str) -> Result<(), String> {
    let tokens = tokenize(source).map_err(|d| format!("lex: {:?}", d))?;
    let module = parse_tokens(&tokens).map_err(|d| format!("parse: {:?}", d))?;
    run_module(&module)
}

/// Run Asili source on the virtual board (`ubao`), with the scenario's scripted pin levels.
/// `triples` is the scenario flattened: `saa, pini, hali` repeated, `hali` 0 or non-zero.
/// Pin and clock events go to the page as they happen (`platform::write_event`).
pub fn run_board_source(source: &str, triples: &[f64]) -> Result<(), String> {
    let ingizo = triples
        .chunks_exact(3)
        .map(|c| asili_evaluator::ubao::Ingizo {
            saa: c[0] as i64,
            pini: c[1] as i64,
            hali: c[2] != 0.0,
        })
        .collect();
    asili_evaluator::ubao::anza(ingizo);
    run_source(source)
}

/// Diagnostics for `source` as a JSON array, `[]` when it is clean: each item is
/// `{"line":L,"column":C,"code":"SEM001","message":"..."}` (line and column are 0 when a
/// diagnostic has no position). Lexing, parsing and the semantic checks run in the order the CLI
/// runs them, so the editor cannot disagree with `pata kagua`'s compile front end.
pub fn check_source(source: &str) -> String {
    check_source_with(source, &[])
}

/// As `check_source`, for a file of a project: `modules` are the other modules the project
/// provides (`leta <name>` resolves to them), so their imports are not reported as unknown.
pub fn check_source_with(source: &str, modules: &[String]) -> String {
    let diags = match tokenize(source) {
        Err(d) => d,
        Ok(tokens) => match parse_tokens(&tokens) {
            Err(d) => d,
            Ok(module) => {
                let (functions, constants) = extern_env_from_imports(&module);
                let resolved: std::collections::HashSet<String> = modules.iter().cloned().collect();
                match asili_parser::semantic_check_with_env_and_modules(
                    &module, true, functions, constants, resolved,
                ) {
                    Ok(()) => Vec::new(),
                    Err(d) => d,
                }
            }
        },
    };
    let items: Vec<String> = diags
        .iter()
        .map(|d| {
            let (line, column) = d.span.as_ref().map_or((0, 0), |s| (s.line, s.column));
            let code = serde_json::to_string(d.code).unwrap_or_else(|_| "\"\"".into());
            let message = serde_json::to_string(&d.message).unwrap_or_else(|_| "\"\"".into());
            format!("{{\"line\":{line},\"column\":{column},\"code\":{code},\"message\":{message}}}")
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The source as `pata nadhifu` writes it (the formatter's canonical form, the same library the CLI
/// calls).
pub fn format_source(source: &str) -> String {
    pata_fmt::canonical_format(source)
}

/// The lint findings for `source` as JSON, in the form `check_source` uses: `[{"line","column",
/// "code","message"}]`, `[]` when clean. A source that cannot be linted reports its error at line 0.
pub fn lint_source_json(source: &str) -> String {
    let items: Vec<String> = match pata_lint::lint_source(source) {
        Ok(findings) => findings
            .iter()
            .map(|d| {
                let (line, column) = d.span.as_ref().map_or((0, 0), |s| (s.line, s.column));
                let code = serde_json::to_string(d.code).unwrap_or_else(|_| "\"\"".into());
                let message = serde_json::to_string(&d.message).unwrap_or_else(|_| "\"\"".into());
                format!("{{\"line\":{line},\"column\":{column},\"code\":{code},\"message\":{message}}}")
            })
            .collect(),
        Err(e) => vec![format!(
            "{{\"line\":0,\"column\":0,\"code\":\"LINT\",\"message\":{}}}",
            serde_json::to_string(&e).unwrap_or_else(|_| "\"\"".into())
        )],
    };
    format!("[{}]", items.join(","))
}

/// Lower `module` to bytecode and run its `kuu` as native code (a wasm module the page
/// instantiates beside this one; see `asili_evaluator::nguvu::wasm`).
fn run_module(module: &Module) -> Result<(), String> {
    let program = compile_module_explained(module)?;
    run_bytecode(&program, vec![]).map_err(|e| e.to_string())
}

/// Run a multi-module Asili program from in-memory sources (no filesystem access, so this is the
/// Wasm-appropriate counterpart to `pata/cli`'s disk-based resolver). `modules` maps a module
/// name (as it appears in `leta <name>`) to that module's source text; `entry_name` is the
/// program's entrypoint. Every `leta` target the entrypoint (transitively) names must have an
/// entry in `modules`, or it is silently skipped — same as an unresolved import today.
pub fn run_bundle(entry_name: &str, modules: &HashMap<String, String>) -> Result<(), String> {
    let entry_source = modules
        .get(entry_name)
        .ok_or_else(|| format!("bundle: entry module '{entry_name}' haipo kwenye modules"))?;
    let tokens = tokenize(entry_source).map_err(|d| format!("lex: {:?}", d))?;
    let entrypoint = parse_tokens(&tokens).map_err(|d| format!("parse: {:?}", d))?;

    let mut parsed: HashMap<String, Module> = HashMap::new();
    let mut pending: Vec<String> = entrypoint
        .imports
        .iter()
        .map(|imp| match &imp.path {
            ImportPath::Full(n) => n.clone(),
            ImportPath::Selective { module, .. } => module.clone(),
        })
        .collect();

    while let Some(name) = pending.pop() {
        if parsed.contains_key(&name) {
            continue;
        }
        let Some(source) = modules.get(&name) else {
            continue;
        };
        let tokens = tokenize(source).map_err(|d| format!("lex ({name}): {:?}", d))?;
        let module = parse_tokens(&tokens).map_err(|d| format!("parse ({name}): {:?}", d))?;
        for imp in &module.imports {
            let dep_name = match &imp.path {
                ImportPath::Full(n) => n.clone(),
                ImportPath::Selective { module, .. } => module.clone(),
            };
            if !parsed.contains_key(&dep_name) {
                pending.push(dep_name);
            }
        }
        parsed.insert(name, module);
    }

    let merged = merge_modules(&entrypoint, &parsed);
    run_module(&merged)
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-browser"))]
mod browser_entry {
    use super::{run_bundle, run_source};
    use std::collections::HashMap;
    use wasm_bindgen::prelude::*;

    /// Single-module entry point for `wasm-bindgen` consumers (e.g. a browser playground).
    #[wasm_bindgen]
    pub fn run(source: &str) -> Result<(), JsValue> {
        run_source(source).map_err(|e| JsValue::from_str(&e))
    }

    /// Diagnostics for `source` as JSON (see `check_source`), for the editor's error marks.
    #[wasm_bindgen]
    pub fn check(source: &str) -> String {
        super::check_source(source)
    }

    /// As `check`, with `modules` a comma-separated list of the project's other module names.
    #[wasm_bindgen(js_name = checkWithModules)]
    pub fn check_with_modules(source: &str, modules: &str) -> String {
        let names: Vec<String> = modules
            .split(',')
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        super::check_source_with(source, &names)
    }

    /// The formatted source (`pata nadhifu`'s canonical form).
    #[wasm_bindgen]
    pub fn format(source: &str) -> String {
        super::format_source(source)
    }

    /// The lint findings as JSON (see `lint_source_json`).
    #[wasm_bindgen]
    pub fn lint(source: &str) -> String {
        super::lint_source_json(source)
    }

    /// Run on the virtual board: `triples` is the scenario (see `run_board_source`).
    #[wasm_bindgen(js_name = runBoard)]
    pub fn run_board(source: &str, triples: &[f64]) -> Result<(), JsValue> {
        super::run_board_source(source, triples).map_err(|e| JsValue::from_str(&e))
    }

    /// Multi-module entry point. `modules` is a JS object/Map-like value convertible to
    /// `HashMap<String, String>` (module name -> source) via `serde-wasm-bindgen` on the host
    /// side; here it arrives already converted for simplicity of the Rust API surface.
    #[wasm_bindgen(js_name = runBundle)]
    pub fn run_bundle_js(entry_name: &str, modules: JsValue) -> Result<(), JsValue> {
        let modules: HashMap<String, String> = serde_wasm_bindgen::from_value(modules)
            .map_err(|e| JsValue::from_str(&format!("modules: {e}")))?;
        run_bundle(entry_name, &modules).map_err(|e| JsValue::from_str(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::{run_bundle, run_source};
    use std::collections::HashMap;

    #[test]
    fn run_source_minimal_kuu() {
        let src = r#"
kazi kuu(hoja: Orodha<Neno>) -> Tupu {
  rejesha
}
"#;
        run_source(src).expect("run_source minimal kazu kuu");
    }

    #[test]
    fn run_bundle_resolves_multi_module_leta() {
        let mut modules = HashMap::new();
        modules.insert(
            "kuu".to_string(),
            "leta mathutil\numma umbo Punkt { x: Namba }\nkazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka p = Punkt { x: PI }\n  rejesha\n}".to_string(),
        );
        modules.insert(
            "mathutil".to_string(),
            "thabiti PI: Namba = 3.14\n".to_string(),
        );

        run_bundle("kuu", &modules).expect("run_bundle resolves cross-module constant");
    }

    #[test]
    fn run_bundle_missing_entry_is_an_error() {
        let modules = HashMap::new();
        let err = run_bundle("kuu", &modules).unwrap_err();
        assert!(err.contains("kuu"));
    }
}

#[cfg(test)]
mod check_tests {
    use super::{check_source, check_source_with};

    #[test]
    fn project_module_imports_resolve() {
        let src = "leta msaidizi\n\nkazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  chapisha(\"hi\")\n}\n";
        assert!(check_source(src).contains("\"code\":\"SEM"), "unknown module is reported");
        let names = vec!["msaidizi".to_string()];
        assert_eq!(check_source_with(src, &names), "[]");
    }

    #[test]
    fn clean_program_has_no_diagnostics() {
        let src = "leta matumizi\n\nkazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  chapisha(\"hi\")\n}\n";
        assert_eq!(check_source(src), "[]");
    }

    #[test]
    fn semantic_error_has_line_code_and_message() {
        let src = "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka x: Namba = \"nope\"\n}\n";
        let out = check_source(src);
        assert!(out.starts_with("[{\"line\":"), "{out}");
        assert!(out.contains("\"code\":\"SEM"), "{out}");
        assert!(out.contains("\"message\":"), "{out}");
    }

    #[test]
    fn parse_error_is_reported_with_position() {
        let out = check_source("kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka = 3\n}\n");
        assert!(out.contains("\"code\":\"PAR"), "{out}");
        assert!(!out.contains("\"line\":0,"), "{out}");
    }
}

#[cfg(test)]
mod fmt_lint_tests {
    use super::{format_source, lint_source_json};

    #[test]
    fn format_is_the_canonical_form_and_idempotent() {
        let messy = "kazi   kuu(hoja:Orodha<Neno>)->Tupu{\nchapisha(\"a\")\n}\n";
        let once = format_source(messy);
        assert!(once.contains("kazi kuu(hoja: Orodha<Neno>) -> Tupu {"), "{once}");
        assert_eq!(format_source(&once), once, "formatting twice changes nothing");
    }

    #[test]
    fn lint_reports_findings_as_json() {
        let clean = "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n}\n";
        assert_eq!(lint_source_json(clean).starts_with('['), true);
        let out = lint_source_json("kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka x = 1\n}\n");
        assert!(out.starts_with('[') && out.contains("\"code\":\"LINT"), "{out}");
    }
}
