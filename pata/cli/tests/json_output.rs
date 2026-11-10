use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestProject(PathBuf);

impl TestProject {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pata-json-output-{stamp}"));
        fs::create_dir_all(root.join("src")).expect("create source directory");
        fs::create_dir_all(root.join("lib/std")).expect("create standard-library directory");
        fs::write(
            root.join("pata.toml"),
            "[jumla]\njina = \"app\"\ntoleo = \"0.1.0\"\nasili = \"1.1\"\n\n[chanzo]\nkuingia = \"src/kuu.as\"\n\n[tegemezi]\n",
        )
        .expect("write project manifest");
        fs::write(
            root.join("src/kuu.as"),
            "leta mfumo\nleta matumizi\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { }\n#[jaribio]\nkazi t_json_1() -> Tupu { chapisha(\"test-output-one\") }\n#[jaribio]\nkazi t_json_2() -> Tupu { chapisha(\"test-output-two\") }\n",
        )
        .expect("write test source");
        fs::write(
            root.join("lib/std/mfumo.asi"),
            "kazi chapisha(ujumbe: Neno) -> Tupu\n",
        )
        .expect("write stdlib stub");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn jaribu_json_keeps_program_output_off_stdout() {
    let project = TestProject::new();
    let output = Command::new(env!("CARGO_BIN_EXE_pata-cli"))
        .args(["jaribu", "--json", "--nyuzi-za-jaribio", "2"])
        .current_dir(project.path())
        .env_remove("ASILI_FUATILIA")
        .output()
        .expect("run pata jaribu");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON report");
    assert_eq!(report["jumla"], 2);
    assert_eq!(report["sawa"], 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("test-output-one"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("test-output-two"));
}

#[test]
fn jenga_json_reports_artifacts_and_structured_option_errors() {
    let project = TestProject::new();
    let executable = env!("CARGO_BIN_EXE_pata-cli");
    let output = Command::new(executable)
        .args(["jenga", "--json"])
        .current_dir(project.path())
        .env_remove("ASILI_FUATILIA")
        .output()
        .expect("run pata jenga");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON build report");
    let artifact = report["artifact"].as_str().expect("artifact path");
    assert!(artifact.ends_with("app.asb"));
    assert!(project.path().join(artifact).is_file());
    assert!(!report["artifacts"].as_array().expect("artifact paths").is_empty());

    let failure = Command::new(executable)
        .args(["jenga", "--haijulikani", "--json"])
        .current_dir(project.path())
        .env_remove("ASILI_FUATILIA")
        .output()
        .expect("run pata jenga with invalid option");
    assert_eq!(failure.status.code(), Some(2));
    assert!(failure.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&failure.stdout).expect("stdout is one JSON error report");
    assert_eq!(report["sawa"], false);
    assert_eq!(report["makosa"][0]["msimbo"], 2);
}
