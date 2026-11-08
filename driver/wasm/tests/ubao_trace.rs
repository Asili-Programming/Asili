//! Trace equality for the virtual board: the same program and scenario (`ubao_trace/`) give the
//! same event lines here and in the browser build, which `tests/ubao_trace.mjs` checks under Node
//! against the same `ubao_trace/expected.txt`.

use std::fs;

fn read(name: &str) -> String {
    let path = format!("{}/tests/ubao_trace/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(path).expect("trace fixture")
}

fn scenario() -> Vec<f64> {
    read("scenario.txt")
        .split_whitespace()
        .map(|t| t.parse::<f64>().expect("number"))
        .collect()
}

#[test]
fn board_events_match_the_fixture() {
    asili_wasm::run_board_source(&read("prova.as"), &scenario()).expect("run");
    let lines: Vec<String> = asili_evaluator::ubao::tukio()
        .iter()
        .map(|t| t.to_json())
        .collect();
    let expected = read("expected.txt");
    let expected: Vec<&str> = expected.lines().collect();
    if std::env::var_os("ASILI_GOLDEN").is_some_and(|v| v == "write") {
        fs::write(
            format!("{}/tests/ubao_trace/expected.txt", env!("CARGO_MANIFEST_DIR")),
            lines.join("\n") + "\n",
        )
        .expect("write fixture");
        return;
    }
    assert_eq!(lines, expected);
}
