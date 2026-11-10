use super::{CliError, CliResult};
use crate::pipeline::compile::{
    cache_key, compile_project, compile_single_file, emit_build_artifacts_with_output, BuildProfile,
};
use crate::pipeline::performance::{PerformanceMetrics, ScopedTimer};
use crate::pipeline::project::find_workspace_root;
use asili_evaluator::run_artifact;
use std::fs;
use std::path::{Path, PathBuf};

const JENGA_USAGE: &str = r#"matumizi: pata jenga [faili.as] [chagua...] [hoja za kuu...]

Jenga mradi kutoka pata.toml au faili moja (bila mradi).

Chagua:
  --tenda            Baada ya kujenga, tenda kazi kuu na hoja zinazofuata.
  --pato <njia>      Mahali pa kuweka kilele (default: kilele/).
  --namna <dev|release>        dev (chaguo-msingi): msimbo asilia ukiwezekana, vinginevyo bila.
                                release: lazima bytecode na msimbo asilia, la sivyo kosa.
  --lengo <lengo>    Lengo la kujenga: "native" (chaguo-msingi), "wasm", "cortex-m" au
                     "esp32" (haya mawili pia huandika kazi za #[salama] kama
                     kilele/<jina>-<lengo>.o na kilele/<jina>.h kwa kifaa). Hupita
                     [jenga] lengo katika pata.toml.
  --workspace-info   Onyesha wanachama wa workspace na urejeshi.
  --json             Toa matokeo ya ujenzi kama JSON.
  --muda             Onyesha muda wa kila awamu ya ujenzi (kuchanganua/kutoa).
  --fuatilia[=namna] Fuatilia ujenzi na utekelezaji: mti (chaguo-msingi), json,
                     json:<faili> au binari:<faili> (pia ASILI_FUATILIA).
  --msaada           Onyesha ujumbe huu.

Hoja za kuu: Baada ya --tenda, hoja zinazofuata zinapewa kwa kuu(hoja: Orodha<Neno>).
Chaguo lisilojulikana ni kosa.

Mifano:
  pata jenga
  pata jenga --tenda
  pata jenga --tenda foo bar
  pata jenga script.as --tenda
  pata jenga --workspace-info
"#;

// Contract: ../../commands/jenga.md
pub fn run(args: &[String]) -> CliResult {
    if args.iter().any(|a| a == "--msaada") {
        print!("{JENGA_USAGE}");
        return Ok(());
    }
    let command_args = args.iter().take_while(|arg| arg.as_str() != "--tenda");
    let json = command_args.clone().any(|a| a == "--json");
    if json
        && (args.iter().any(|arg| arg == "--tenda")
            || command_args
                .clone()
                .any(|arg| arg == "--fuatilia" || arg.starts_with("--fuatilia=")))
    {
        return Err(CliError::new(
            "--json haiwezi kutumiwa pamoja na --tenda au --fuatilia",
            2,
        ));
    }
    if command_args.clone().any(|a| a == "--workspace-info") {
        if let Some(unknown) = args
            .iter()
            .take_while(|arg| arg.as_str() != "--tenda")
            .find(|arg| !matches!(arg.as_str(), "--workspace-info" | "--json"))
        {
            return Err(CliError::new(
                format!("hoja isiyotambuliwa kwenye jenga: {unknown}"),
                2,
            ));
        }
        show_workspace_info(json)?;
        return Ok(());
    }
    let (profile, out, do_run, single_file, program_args, build_target, show_timing, json) =
        parse_args(args)?;
    let profile = BuildProfile::parse(&profile)?;
    let mut metrics = PerformanceMetrics::new(5000);
    let total_timer = ScopedTimer::new("jumla");
    if let Some(ref path) = single_file {
        if !path.exists() {
            return Err(CliError::new(format!("faili haipo: {}", path.display()), 1));
        }
        if !path.is_file() {
            return Err(CliError::new(
                format!("si faili: {} (inahitaji faili .as)", path.display()),
                1,
            ));
        }
    }

    let (root, compiled) = if let Some(ref path) = single_file {
        let root = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let target = out.clone().unwrap_or_else(|| root.join("kilele"));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("script");
        let source = fs::read_to_string(path)
            .map_err(|e| CliError::new(format!("imeshindwa kusoma {}: {e}", path.display()), 1))?;
        let key = cache_key(name, &source, build_target.as_deref().unwrap_or("native"));
        let cache_path = target.join(".asb-cache").join(format!("{key}.asb"));
        // The cache only ever holds best-effort artifacts; release always rebuilds.
        if profile == BuildProfile::Dev && cache_path.exists() {
            fs::create_dir_all(&target).map_err(|e| {
                CliError::new(format!("imeshindwa kuunda {}: {e}", target.display()), 1)
            })?;
            let asb_bytes = fs::read(&cache_path).map_err(|e| {
                CliError::new(
                    format!("imeshindwa kusoma cache {}: {e}", cache_path.display()),
                    1,
                )
            })?;
            // An artifact this build cannot read is rebuilt, not an error.
            let cached = asili_evaluator::load_asb_bytecode(&asb_bytes)
                .ok()
                .and_then(|program| program.ast);
            if let Some(module) = cached {
                let artifact = target.join(format!("{name}.asb"));
                fs::write(&artifact, &asb_bytes).map_err(|e| {
                    CliError::new(
                        format!("imeshindwa kuandika {}: {e}", artifact.display()),
                        1,
                    )
                })?;
                let meta = target.join(format!("{name}.build.manifest"));
                let manifest = format!(
                    "mradi={name}\nkuingia={}\nkazi={}\nkilele={name}.asb\n",
                    path.display(),
                    module.functions.len()
                );
                let _ = fs::write(&meta, manifest);
                if !json {
                    println!("imejengwa (cache): {}", artifact.display());
                }
                if do_run {
                    run_artifact(&artifact, program_args)
                        .map_err(|e| CliError::new(e.to_string(), e.exit_code()))?;
                }
                if show_timing {
                    metrics.set_total(total_timer.elapsed());
                    if json {
                        print_build_json(
                            name,
                            path,
                            build_target.as_deref().unwrap_or("native"),
                            &artifact,
                            &target,
                            true,
                            Some(metrics.report()),
                        );
                    } else {
                        println!("{}", metrics.report());
                    }
                } else if json {
                    print_build_json(
                        name,
                        path,
                        build_target.as_deref().unwrap_or("native"),
                        &artifact,
                        &target,
                        true,
                        None,
                    );
                }
                return Ok(());
            }
        }
        let compile_timer = ScopedTimer::new("kuchanganua");
        let compiled = compile_single_file(path, build_target.as_deref())?;
        metrics.record_phase(compile_timer.name(), compile_timer.elapsed());
        (root, compiled)
    } else {
        let root = Path::new(".");
        let compile_timer = ScopedTimer::new("kuchanganua");
        let compiled = compile_project(root, build_target.as_deref())?;
        metrics.record_phase(compile_timer.name(), compile_timer.elapsed());
        (root.to_path_buf(), compiled)
    };

    let emit_timer = ScopedTimer::new("kutoa");
    // A cached build reuses only the bytecode: a device target also writes its object and
    // header, which the cache does not hold, so it always emits.
    let device = crate::pipeline::compile::device_lengo(&compiled.target.0).is_some();
    let artifact = if compiled.from_cache && profile == BuildProfile::Dev && !device {
        let target = out.as_ref().cloned().unwrap_or_else(|| root.join("kilele"));
        let a = target.join(format!("{}.asb", compiled.config.name));
        if !json {
            println!("imejengwa (cache): {}", a.display());
        }
        a
    } else {
        let a = emit_build_artifacts_with_output(&root, &compiled, out.as_deref(), profile, !json)?;
        if !json {
            println!("imejengwa: {}", a.display());
        }
        a
    };
    metrics.record_phase(emit_timer.name(), emit_timer.elapsed());

    if do_run {
        // Run what was just built (bytecode + native code when available), exactly as
        // `pata tenda` would, rather than re-interpreting the in-memory AST.
        run_artifact(&artifact, program_args)
            .map_err(|e| CliError::new(e.to_string(), e.exit_code()))?;
    }

    if json {
        print_build_json(
            &compiled.config.name,
            &compiled.source_path,
            &compiled.target.0,
            &artifact,
            out.as_deref().unwrap_or(&root.join("kilele")),
            compiled.from_cache,
            show_timing.then(|| {
                metrics.set_total(total_timer.elapsed());
                metrics.report()
            }),
        );
    } else if show_timing {
        metrics.set_total(total_timer.elapsed());
        println!("{}", metrics.report());
    }
    Ok(())
}

fn show_workspace_info(json: bool) -> CliResult {
    let cwd = PathBuf::from(".");
    match find_workspace_root(&cwd) {
        Some(ws) => {
            if json {
                let output = serde_json::json!({
                    "sawa": true,
                    "root": ws.root,
                    "wanachama": ws.members.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                println!("Eneo-kazi: {}", ws.root.display());
                println!("Wanachama: {}", ws.members.len());
                for (name, _manifest) in &ws.members {
                    println!("  - {}", name);
                }
            }
            Ok(())
        }
        None => Err(CliError::new(
            "eneo-kazi haipo (pata.toml haina jedwali [eneo-kazi])",
            1,
        )),
    }
}

fn print_build_json(
    name: &str,
    source: &Path,
    target: &str,
    artifact: &Path,
    output_dir: &Path,
    from_cache: bool,
    timing: Option<String>,
) {
    let output = build_json_value(
        name, source, target, artifact, output_dir, from_cache, timing,
    );
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn build_json_value(
    name: &str,
    source: &Path,
    target: &str,
    artifact: &Path,
    output_dir: &Path,
    from_cache: bool,
    timing: Option<String>,
) -> serde_json::Value {
    let mut artifacts = vec![artifact.to_path_buf()];
    let mut optional = vec![
        output_dir.join(format!("{name}.build.manifest")),
        output_dir.join(asili_evaluator::nguvu::image_file_name(name)),
        output_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
    ];
    if let Some(device) = crate::pipeline::compile::device_lengo(target) {
        let _ = device;
        optional.push(output_dir.join(format!("{name}-{target}.o")));
        optional.push(output_dir.join(format!("{name}.h")));
    }
    artifacts.extend(optional.into_iter().filter(|path| path.is_file()));
    artifacts.sort();
    artifacts.dedup();
    serde_json::json!({
        "sawa": true,
        "mradi": name,
        "chanzo": source,
        "lengo": target,
        "artifact": artifact,
        "artifacts": artifacts,
        "cache": from_cache,
        "ujumbe": timing.into_iter().collect::<Vec<_>>(),
        "makosa": [],
    })
}

#[allow(clippy::type_complexity)]
pub fn parse_args(
    args: &[String],
) -> Result<
    (
        String,
        Option<PathBuf>,
        bool,
        Option<PathBuf>,
        Vec<String>,
        Option<String>,
        bool,
        bool,
    ),
    CliError,
> {
    let mut profile = String::from("dev");
    let mut out = None;
    let mut do_run = false;
    let mut single_file = None;
    let mut program_args = Vec::new();
    let mut target = None;
    let mut show_timing = false;
    let mut json = false;
    let mut program_args_mode = false;
    let mut i = 0usize;
    while i < args.len() {
        if program_args_mode {
            program_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--namna" => {
                let Some(p) = args.get(i + 1) else {
                    return Err(CliError::new("--namna inahitaji thamani", 2));
                };
                profile = p.clone();
                i += 2;
            }
            "--pato" => {
                let Some(p) = args.get(i + 1) else {
                    return Err(CliError::new("--pato inahitaji njia", 2));
                };
                out = Some(PathBuf::from(p));
                i += 2;
            }
            "--lengo" => {
                let Some(t) = args.get(i + 1) else {
                    return Err(CliError::new("--lengo inahitaji thamani", 2));
                };
                target = Some(t.clone());
                i += 2;
            }
            "--tenda" => {
                do_run = true;
                program_args_mode = true;
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            "--muda" => {
                show_timing = true;
                i += 1;
            }
            other => {
                if let Some(traced) = super::trace_flag(other) {
                    traced?;
                } else if other.ends_with(".as") && single_file.is_none() {
                    single_file = Some(PathBuf::from(other));
                } else if other == "--" && do_run {
                    program_args_mode = true;
                } else {
                    return Err(CliError::new(
                        format!("hoja isiyotambuliwa kwenye jenga: {other}"),
                        2,
                    ));
                }
                i += 1;
            }
        }
    }
    Ok((
        profile,
        out,
        do_run,
        single_file,
        program_args,
        target,
        show_timing,
        json,
    ))
}

#[cfg(test)]
mod tests {
    use super::{build_json_value, parse_args, run};
    use crate::commands::TEST_CWD_LOCK;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn emits_asb_artifact() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        std::env::set_current_dir(&root).expect("chdir");
        run(&[]).expect("jenga ok");
        let asb_bytes = fs::read("kilele/app.asb").expect("artifact");
        assert!(
            asb_bytes.starts_with(b"ASB-STUB"),
            "asb should have ASB-STUB header"
        );
        let functions = asili_evaluator::load_asb_bytecode(&asb_bytes)
            .expect("load_asb_bytecode")
            .functions
            .len();
        assert!(functions > 0, "asb should contain merged module");
        let manifest =
            fs::read_to_string("kilele/app.build.manifest").expect("per-artifact manifest");
        assert!(
            manifest.contains("mradi=app"),
            "manifest should have mradi=app"
        );
        assert!(manifest.contains("kuingia="));
        assert!(manifest.contains("kazi="));
        assert!(
            manifest.contains("kilele=app.asb"),
            "manifest should point at .asb"
        );
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

    /// Real end-to-end: `pata jenga --muda` must build successfully and go through
    /// `PerformanceMetrics::record_phase`/`report()` without panicking — the actual wiring
    /// gap this test closes (that code path was entirely uncalled from any command before).
    /// `report()`'s own formatting (phase names, SLO line) is covered directly by
    /// `pipeline::performance`'s unit tests; this test's job is proving the CLI path reaches it.
    #[test]
    fn muda_flag_builds_successfully_and_does_not_panic() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        std::env::set_current_dir(&root).expect("chdir");

        let result = run(&["--muda".to_string()]);

        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);

        result.expect("jenga --muda should build successfully and print timing");
    }

    #[test]
    fn json_report_has_machine_readable_artifact_paths() {
        let root = temp_project();
        let output_dir = root.join("kilele");
        fs::create_dir_all(&output_dir).expect("output directory");
        let artifact = output_dir.join("app.asb");
        let manifest = output_dir.join("app.build.manifest");
        fs::write(&artifact, b"artifact").expect("artifact");
        fs::write(&manifest, "manifest").expect("manifest");

        let report = build_json_value(
            "app",
            &root.join("src/kuu.as"),
            "native",
            &artifact,
            &output_dir,
            false,
            Some("muda".to_string()),
        );

        assert_eq!(report["sawa"], true);
        assert_eq!(report["mradi"], "app");
        assert_eq!(report["lengo"], "native");
        assert_eq!(report["cache"], false);
        assert_eq!(report["artifact"], artifact.to_string_lossy().as_ref());
        assert!(report["artifacts"]
            .as_array()
            .expect("artifact paths")
            .iter()
            .any(|path| path == &manifest.to_string_lossy().as_ref()));
        assert_eq!(report["ujumbe"][0], "muda");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn jenga_rejects_unknown_options_but_passes_run_arguments() {
        let unknown = parse_args(&["--haijulikani".to_string()])
            .expect_err("unknown flags must not be forwarded silently");
        assert_eq!(unknown.exit_code, 2);

        let parsed = parse_args(&[
            "--tenda".to_string(),
            "--flag-ya-programu".to_string(),
            "thamani".to_string(),
        ])
        .expect("program arguments after --tenda are forwarded");
        assert!(parsed.2);
        assert_eq!(parsed.4, ["--flag-ya-programu", "thamani"]);
    }

    #[test]
    fn muda_flag_works_with_single_file_cache_hit_path() {
        // --muda must also work on the single-file cache-hit early-return path (a separate code
        // path from the normal project build, with its own metrics.report() call) -- build once
        // to populate the cache, then again with --muda to exercise that specific branch.
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        std::env::set_current_dir(&root).expect("chdir");

        run(&["src/kuu.as".to_string()]).expect("first single-file build");
        let result = run(&["src/kuu.as".to_string(), "--muda".to_string()]);

        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);

        result.expect("cached single-file jenga --muda should succeed and print timing");
    }

    #[test]
    fn two_builds_produce_two_manifests_no_overwrite() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        std::env::set_current_dir(&root).expect("chdir");
        run(&[]).expect("jenga project ok");
        fs::write(
            root.join("other.as"),
            "leta mfumo\nleta matumizi\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { chapisha(\"y\") }",
        )
        .expect("write other.as");
        run(&["other.as".into()]).expect("jenga single file ok");
        let app_manifest = fs::read_to_string("kilele/app.build.manifest").expect("app manifest");
        let other_manifest =
            fs::read_to_string("kilele/other.build.manifest").expect("other manifest");
        assert!(app_manifest.contains("mradi=app") && app_manifest.contains("kilele=app.asb"));
        assert!(
            other_manifest.contains("mradi=other") && other_manifest.contains("kilele=other.asb")
        );
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

    fn temp_project() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pata-jenga-{stamp}"));
        fs::create_dir_all(dir.join("src")).expect("mkdir");
        fs::create_dir_all(dir.join("lib/std")).expect("mkdir lib/std");
        fs::write(
            dir.join("pata.toml"),
            "[jumla]\njina = \"app\"\ntoleo = \"0.1.0\"\nasili = \"1.1\"\n\n[chanzo]\nkuingia = \"src/kuu.as\"\n\n[tegemezi]\n",
        )
        .expect("write manifest");
        fs::write(
            dir.join("src/kuu.as"),
            "leta mfumo\nleta matumizi\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { chapisha(\"x\") }",
        )
        .expect("src");
        fs::write(
            dir.join("lib/std/mfumo.asi"),
            "kazi chapisha(ujumbe: Neno) -> Tupu\n",
        )
        .expect("stdlib mfumo");
        dir
    }

    #[test]
    fn jenga_run_executes_kuu() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        std::env::set_current_dir(&root).expect("chdir");
        run(&["--tenda".into()]).expect("jenga --tenda ok");
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn jenga_resolves_builtin_hisabati_without_file() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        fs::write(
            root.join("src/kuu.as"),
            "leta hisabati\nleta matumizi\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { weka x = jumla(2, 3) chapisha(\"ok\") }",
        )
        .expect("write");
        std::env::set_current_dir(&root).expect("chdir");
        run(&[]).expect("jenga with leta hisabati (builtin, no lib/std/hisabati.asi)");
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn jenga_core_complete_struct_impl_index_kamusi() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let root = temp_project();
        let src = r#"leta mfumo
leta matumizi
umbo Pika { x: Namba }
shughuli ya Pika { kazi ongeza(self: Pika, d: Namba) -> Namba { rejesha self.x + d } }
kazi kuu(hoja: Orodha<Neno>) -> Tupu {
  weka p = Pika { x: 5 }
  weka n = p.ongeza(3)
  chapisha("ok")
}"#;
        fs::write(root.join("src/kuu.as"), src).expect("write");
        std::env::set_current_dir(&root).expect("chdir");
        run(&["--tenda".into()]).expect("jenga --tenda with struct and impl");
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn jenga_single_file_tenda() {
        let _guard = TEST_CWD_LOCK.lock().expect("lock");
        let original = std::env::current_dir().expect("cwd");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pata-jenga-single-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");
        let script = dir.join("one.as");
        fs::write(
            &script,
            "leta mfumo\nleta matumizi\nkazi kuu(hoja: Orodha<Neno>) -> Tupu { chapisha(\"x\") }",
        )
        .expect("write");
        std::env::set_current_dir(&dir).expect("chdir");
        let path_str = script.to_string_lossy().to_string();
        run(&[path_str, "--tenda".into()]).expect("jenga single file --tenda ok");
        std::env::set_current_dir(&original).expect("restore cwd");
        let _ = fs::remove_dir_all(&dir);
    }
}
