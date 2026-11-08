//! The editors' word lists — the VS Code grammar (`syntaxes/asili.tmLanguage.json` in
//! `Asili-Programming/asili-vscode`) and the browser playground's highlighter (`main.js` in
//! `Asili-Programming/asili-playground`) — are generated from the compiler's own lists, so they
//! cannot drift: keywords from `asili_lexer::KEYWORDS`, types from
//! `asili_parser::builtins::BUILTIN_TYPE_NAMES` plus the builtin `umbo`s, builtin functions from
//! the builtin table.
//!
//! Those files live in their own repositories, so this test finds them through
//! `ASILI_VSCODE_DIR` and `ASILI_PLAYGROUND_DIR` (each repository's root). Each of those
//! repositories' CI checks out Asili and runs this test against itself; with neither variable set
//! there is nothing to check here. It fails while a file differs; `ASILI_GOLDEN=write` rewrites
//! them:
//!
//! ```text
//! ASILI_VSCODE_DIR=../asili-vscode ASILI_PLAYGROUND_DIR=../asili-playground \
//!     ASILI_GOLDEN=write cargo test -p pata-core --test editor_word_lists
//! ```

use asili_parser::builtins::{
    builtin_module_exports, builtin_structs, BUILTIN_MODULE_NAMES, BUILTIN_TYPE_NAMES,
};
use std::path::{Path, PathBuf};

/// The repository root named by `var`, relative paths taken from the Asili checkout's root.
fn repo(var: &str) -> Option<PathBuf> {
    let dir = std::env::var_os(var)?;
    Some(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(dir))
}

fn types() -> Vec<String> {
    let mut t: Vec<String> = BUILTIN_TYPE_NAMES.iter().map(|s| s.to_string()).collect();
    t.extend(builtin_structs().iter().map(|s| s.name.clone()));
    t
}

fn functions() -> Vec<String> {
    let mut f: Vec<String> = BUILTIN_MODULE_NAMES
        .iter()
        .filter_map(|m| builtin_module_exports(m))
        .flat_map(|t| t.functions.into_keys())
        .collect();
    f.sort();
    f.dedup();
    f
}

/// Replace the value of the first `"key": "..."` after `anchor` with `value` (JSON-escaped).
fn set_json_string(text: &str, anchor: &str, key: &str, value: &str) -> String {
    let at = text
        .find(anchor)
        .unwrap_or_else(|| panic!("{anchor} not in grammar"));
    let start = at + text[at..].find(&format!("\"{key}\": \"")).expect("key") + key.len() + 5;
    let bytes = text.as_bytes();
    let mut end = start;
    while bytes[end] != b'"' || bytes[end - 1] == b'\\' {
        end += 1;
    }
    format!(
        "{}{}{}",
        &text[..start],
        value.replace('\\', "\\\\"),
        &text[end..]
    )
}

fn js_set(name: &str, words: &[String]) -> String {
    let mut out = format!("const {name} = new Set([");
    let mut line = String::new();
    for w in words {
        let item = format!("\"{w}\", ");
        if line.len() + item.len() > 96 {
            out.push_str(&format!("\n  {}", line.trim_end()));
            line.clear();
        }
        line.push_str(&item);
    }
    if !line.is_empty() {
        out.push_str(&format!("\n  {}", line.trim_end()));
    }
    out.push_str("\n]);");
    out
}

fn check_grammar(grammar_path: &Path, write: bool, stale: &mut Vec<String>) {
    let grammar = std::fs::read_to_string(grammar_path)
        .unwrap_or_else(|e| panic!("{}: {e}", grammar_path.display()));
    let mut want = set_json_string(
        &grammar,
        "\"builtin-type\": {",
        "match",
        &format!("\\b({})\\b", types().join("|")),
    );
    want = set_json_string(
        &want,
        "\"builtin-function\": {",
        "begin",
        &format!("\\b({})\\s*(\\()", functions().join("|")),
    );
    if want != grammar {
        if write {
            std::fs::write(grammar_path, &want).expect("write grammar");
        } else {
            stale.push(grammar_path.display().to_string());
        }
    }
    // Every keyword is highlighted by some keyword pattern of the grammar.
    for kw in asili_lexer::KEYWORDS {
        let listed = ["(", "|"].iter().any(|before| {
            ["|", ")"]
                .iter()
                .any(|after| want.contains(&format!("{before}{kw}{after}")))
        });
        assert!(
            listed,
            "keyword `{kw}` is not in any pattern of asili.tmLanguage.json"
        );
    }
}

fn check_playground(js_path: &Path, write: bool, stale: &mut Vec<String>) {
    let js =
        std::fs::read_to_string(js_path).unwrap_or_else(|e| panic!("{}: {e}", js_path.display()));
    let start = js.find("const KEYWORDS = new Set(").expect("KEYWORDS");
    let end = js[start..].find("// </generated>").expect("marker") + start;
    let mut kws: Vec<String> = asili_lexer::KEYWORDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    kws.sort();
    let block = format!(
        "{}\n{}\n",
        js_set("KEYWORDS", &kws),
        js_set("BUILTIN_TYPES", &types())
    );
    let want_js = format!("{}{}{}", &js[..start], block, &js[end..]);
    if want_js != js {
        if write {
            std::fs::write(js_path, &want_js).expect("write main.js");
        } else {
            stale.push(js_path.display().to_string());
        }
    }
}

#[test]
fn editor_word_lists_are_generated() {
    let write = std::env::var_os("ASILI_GOLDEN").is_some_and(|v| v == "write");
    let mut stale = Vec::new();

    if let Some(dir) = repo("ASILI_VSCODE_DIR") {
        check_grammar(
            &dir.join("syntaxes/asili.tmLanguage.json"),
            write,
            &mut stale,
        );
    }
    if let Some(dir) = repo("ASILI_PLAYGROUND_DIR") {
        check_playground(&dir.join("main.js"), write, &mut stale);
    }
    assert!(
        stale.is_empty(),
        "editor word lists differ from the compiler's; run \
         `ASILI_GOLDEN=write cargo test -p pata-core --test editor_word_lists` with \
         ASILI_VSCODE_DIR / ASILI_PLAYGROUND_DIR set:\n{}",
        stale.join("\n")
    );
}
