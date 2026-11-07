//! `pata kagua [njia]`: lint `.as` sources with `pata-lint`'s rules (`pata_lint`), honoring the
//! nearest `pata.toml`'s `[lint.rules]`.

use super::{CliError, CliResult};
use crate::pipeline::format::collect_asili_files;
use pata_lint::{config::LintConfig, lint_source_with_config};
use std::path::{Path, PathBuf};

// Contract: ../../commands/kagua.md
const KAGUA_USAGE: &str = r#"matumizi: pata kagua [njia] [chagua...]

Kagua mtindo, majina na mazoea ya chanzo cha Asili (faili moja au saraka nzima; chaguo-msingi ni
saraka ya sasa). Sheria husomwa kutoka [lint.rules] ya pata.toml iliyo karibu zaidi.

Chagua:
  --makosa-tu   Ripoti makosa tu, si maonyo ya ukaguzi.
  --msaada      Onyesha ujumbe huu.

Inatoka na 1 ikiwa faili haikuweza kukaguliwa, au (bila --makosa-tu) ukaguzi ukipata tatizo.
"#;

pub fn run(args: &[String]) -> CliResult {
    if args.iter().any(|a| a == "--msaada") {
        print!("{KAGUA_USAGE}");
        return Ok(());
    }
    let (errors_only, path) = parse_args(args)?;
    let path = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    // `[lint.rules]` is read from the nearest ancestor `pata.toml`, walking up from `path`. No
    // `pata.toml` (or no `[lint.rules]`) lints with every rule at its default.
    let config = LintConfig::find_and_load(&path).unwrap_or_else(|e| {
        eprintln!("Onyo: imeshindwa kusoma usanidi wa pata.toml: {e}");
        LintConfig::new()
    });

    let files = if path.is_file() {
        vec![path.clone()]
    } else if path.is_dir() {
        // `.asi` files are interface stubs — bare signatures with no body, which the full `.as`
        // grammar rejects — and hold nothing a style/naming linter checks.
        collect_asili_files(&path)?
            .into_iter()
            .filter(|f| f.extension().is_some_and(|e| e == "as"))
            .collect()
    } else {
        return Err(CliError::new(
            format!("{} si faili wala saraka", path.display()),
            2,
        ));
    };

    let report = lint_files(&files, errors_only, &config);
    if report.failed > 0 {
        return Err(CliError::new(
            format!("faili zilizoshindwa kukaguliwa: {}", report.failed),
            1,
        ));
    }
    if report.findings > 0 {
        return Err(CliError::new(
            format!("matatizo yaliyopatikana na ukaguzi: {}", report.findings),
            1,
        ));
    }
    Ok(())
}

/// What linting a set of files found: problems reported, and files that could not be linted.
#[derive(Debug, Default, PartialEq)]
struct Report {
    findings: usize,
    failed: usize,
}

/// Lint each file, printing its findings (or that it is clean) — one unlintable file is
/// reported and the rest still linted.
fn lint_files(files: &[PathBuf], errors_only: bool, config: &LintConfig) -> Report {
    let mut report = Report::default();
    for file in files {
        match lint_file(file, errors_only, config) {
            Ok(n) => report.findings += n,
            Err(e) => {
                eprintln!("Kosa: {e}");
                report.failed += 1;
            }
        }
    }
    report
}

fn lint_file(path: &Path, errors_only: bool, config: &LintConfig) -> Result<usize, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("imeshindwa kusoma {}: {e}", path.display()))?;
    let diags = lint_source_with_config(&source, config)
        .map_err(|e| format!("imeshindwa kukagua {}: {e}", path.display()))?;
    let shown: Vec<_> = diags
        .iter()
        .filter(|d| !(errors_only && d.stage == "ukaguzi"))
        .collect();
    if diags.is_empty() {
        println!("✓ {} - hakuna tatizo lililopatikana", path.display());
    }
    for diag in &shown {
        let location = match &diag.span {
            Some(span) => format!("{}:{}", span.line, span.column),
            None => "1:1".to_string(),
        };
        println!(
            "{}:{} ({}): {}",
            path.display(),
            location,
            diag.code,
            diag.message
        );
    }
    Ok(shown.len())
}

fn parse_args(args: &[String]) -> Result<(bool, Option<String>), CliError> {
    let mut errors_only = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--makosa-tu" => errors_only = true,
            other if !other.starts_with('-') => {
                if path.is_some() {
                    return Err(CliError::new(
                        format!("kagua inakubali njia moja tu, umetoa nyingine: {other}"),
                        2,
                    ));
                }
                path = Some(other.to_string());
            }
            other => {
                return Err(CliError::new(
                    format!("hoja isiyotambuliwa kwenye kagua: {other}"),
                    2,
                ))
            }
        }
    }
    Ok((errors_only, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_project;

    const CLEAN: &str = "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n    chapisha(\"x\")\n}\n";
    // LINT001: a function name that isn't snake_case.
    const BAD_NAME: &str = "kazi JinaBaya() -> Tupu {\n    rejesha Tupu\n}\n";

    fn arg(p: &Path) -> String {
        p.display().to_string()
    }

    #[test]
    fn a_clean_project_passes() {
        let dir = temp_project("kagua-safi", CLEAN);
        run(&[arg(&dir)]).expect("clean source has nothing to report");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_finding_fails_unless_only_errors_are_asked_for() {
        let dir = temp_project("kagua-onyo", BAD_NAME);
        let err = run(&[arg(&dir)]).expect_err("a lint finding fails the command");
        assert_eq!(err.exit_code, 1);
        run(&["--makosa-tu".into(), arg(&dir)]).expect("warnings alone pass with --makosa-tu");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An unparseable file fails even with `--makosa-tu`, and doesn't stop the others being
    /// linted; `.asi` stubs (bodiless signatures) are not linted at all.
    #[test]
    fn an_unlintable_file_fails_and_stubs_are_skipped() {
        let dir = temp_project("kagua-kosa", CLEAN);
        std::fs::write(
            dir.join("src/stub.asi"),
            "kazi chapisha(ujumbe: Neno) -> Tupu\n",
        )
        .unwrap();
        run(&["--makosa-tu".into(), arg(&dir)]).expect("an .asi stub is not linted");

        std::fs::write(dir.join("src/vunjika.as"), "kazi ( {\n").unwrap();
        let files: Vec<_> = collect_asili_files(&dir)
            .unwrap()
            .into_iter()
            .filter(|f| f.extension().is_some_and(|e| e == "as"))
            .collect();
        let report = lint_files(&files, true, &LintConfig::new());
        assert_eq!(
            report,
            Report {
                findings: 0,
                failed: 1
            }
        );
        let err = run(&["--makosa-tu".into(), arg(&dir)]).expect_err("parse failure fails");
        assert_eq!(err.exit_code, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_arguments_are_usage_errors() {
        assert_eq!(run(&["--hakuna".into()]).unwrap_err().exit_code, 2);
        assert_eq!(run(&["a".into(), "b".into()]).unwrap_err().exit_code, 2);
        assert_eq!(run(&["/hakuna/njia/hii".into()]).unwrap_err().exit_code, 2);
    }
}
