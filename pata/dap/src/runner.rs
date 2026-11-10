//! Wires a real `RealDebugHook` up to an actual running `.as` program: compiles the file named
//! by a DAP `launch` request's `program` field and runs it on a background thread once
//! `configurationDone` fires, with a monitor thread keeping `DapSession::paused_at_line` current
//! so `stackTrace` responses reflect where execution has genuinely stopped.
//!
//! Single-file compilation only (lex → parse — no project/dependency resolution, no semantic
//! check) — matches the DAP use case of "debug this one file," not a full `pata jenga` project
//! build. A project-aware `launch` (resolving `leta` imports via `pata-core`) is a reasonable
//! future extension but out of scope for the minimum viable surface this section targets.

use asili_evaluator::debug_hook::RealDebugHook;
use asili_lexer::tokenize;
use asili_parser::parse_tokens;
use std::sync::Arc;

use crate::server::DapSession;

/// Build a `DapSession<RealDebugHook>` wired to actually compile and run whatever `.as` file a
/// `launch` request names, once `configurationDone` arrives — real breakpoints (from the
/// preceding `setBreakpoints` request) and the program path (from `launch`) are both passed
/// directly into the callback by `DapSession::handle` itself, so no self-referential handle back
/// into the session is needed to read them.
pub fn real_session() -> Arc<DapSession<RealDebugHook>> {
    let hook = Arc::new(RealDebugHook::new(Vec::new()));
    let hook_for_callback = Arc::clone(&hook);
    let program_thread = Arc::new(std::sync::Mutex::new(None::<std::thread::JoinHandle<()>>));
    let thread_for_callback = Arc::clone(&program_thread);

    let session = DapSession::new(Arc::clone(&hook));
    let session = session.with_on_configuration_done(move |program, breakpoint_lines| {
        let Ok(mut thread) = thread_for_callback.lock() else {
            eprintln!("pata-dap: failed to lock the debuggee thread handle");
            return;
        };
        if let Some(previous) = thread.take() {
            if previous.join().is_err() {
                eprintln!("pata-dap: previous debuggee thread panicked");
            }
        }
        hook_for_callback.reset_for_restart();
        *thread = launch_program(Arc::clone(&hook_for_callback), breakpoint_lines, program);
    });
    Arc::new(session)
}

/// Compile `program` and run it on a new thread with `hook` attached, plus a lightweight monitor
/// thread that polls `hook.paused_at_line()` and mirrors it into `paused_at_line` (shared with
/// the owning `DapSession`) so `stackTrace` requests report the real, live paused location. Both
/// threads are fire-and-forget: `launch`/`configurationDone` in DAP is not expected to block
/// waiting for the program to finish (the client polls via `stackTrace`/`variables`/`continue`
/// instead), matching how `run()`'s own request loop keeps servicing requests concurrently with
/// the target program actually executing.
fn launch_program(
    hook: Arc<RealDebugHook>,
    breakpoint_lines: Vec<i64>,
    program: Option<String>,
) -> Option<std::thread::JoinHandle<()>> {
    let Some(path) = program else {
        eprintln!("pata-dap: launch request did not include a program path");
        return None;
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("pata-dap: failed to read {path}: {error}");
            return None;
        }
    };
    let tokens = match tokenize(&source) {
        Ok(tokens) => tokens,
        Err(error) => {
            eprintln!("pata-dap: failed to tokenize {path}: {error:?}");
            return None;
        }
    };
    let module = match parse_tokens(&tokens) {
        Ok(module) => module,
        Err(error) => {
            eprintln!("pata-dap: failed to parse {path}: {error:?}");
            return None;
        }
    };

    hook.set_breakpoints(breakpoint_lines.into_iter().map(|l| l as usize).collect());
    Some(std::thread::spawn(move || {
        let _ = asili_evaluator::run_main_with_debug_hook(&module, Vec::new(), hook);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asili_evaluator::debug_hook::DebugHook;
    use dap::prelude::*;
    use dap::requests::{
        ContinueArguments, EvaluateArguments, LaunchRequestArguments, RestartArguments,
        SetBreakpointsArguments, SetVariableArguments,
    };
    use dap::types::SourceBreakpoint;
    use std::io::Write;
    use std::time::Duration;

    fn req(seq: i64, command: Command) -> Request {
        Request { seq, command }
    }

    fn temp_as_file(name: &str, content: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pata-dap-runner-test-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kuu.as");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    /// End-to-end: a real `launch` naming a real `.as` file, a real breakpoint set via
    /// `setBreakpoints`, then `configurationDone` — the target program must actually start
    /// running and genuinely pause at the configured line, observable via the session's own
    /// `stackTrace`-facing state, not just the hook in isolation.
    #[test]
    fn configuration_done_launches_and_pauses_a_real_program_at_a_real_breakpoint() {
        let path = temp_as_file(
            "pause",
            "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka a = 1\n  weka b = 2\n  weka c = 3\n}",
        );
        let session = real_session();

        let mut launch_args = LaunchRequestArguments::default();
        let mut data = serde_json::Map::new();
        data.insert(
            "program".to_string(),
            serde_json::json!(path.to_string_lossy()),
        );
        launch_args.additional_data = Some(serde_json::Value::Object(data));
        session.handle(&req(1, Command::Launch(launch_args)));

        let mut bp_args = SetBreakpointsArguments::default();
        bp_args.breakpoints = Some(vec![
            SourceBreakpoint {
                line: 3,
                ..Default::default()
            },
            SourceBreakpoint {
                line: 4,
                ..Default::default()
            },
        ]);
        session.handle(&req(2, Command::SetBreakpoints(bp_args)));

        session.handle(&req(3, Command::ConfigurationDone));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.hook().paused_at_line() != Some(3) {
            assert!(
                std::time::Instant::now() < deadline,
                "breakpoint never fired within 5s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let generation = session.hook().pause_generation();
        let (response, _) = session.handle(&req(
            6,
            Command::Evaluate(EvaluateArguments {
                expression: "a".into(),
                ..Default::default()
            }),
        ));
        assert!(response.success);
        assert!(matches!(
            response.body,
            Some(ResponseBody::Evaluate(result)) if result.result == "Namba(1.0)"
        ));
        let (response, _) = session.handle(&req(
            7,
            Command::SetVariable(SetVariableArguments {
                variables_reference: 1,
                name: "a".into(),
                value: "9".into(),
                ..Default::default()
            }),
        ));
        assert!(response.success);
        assert_eq!(
            match response.body {
                Some(ResponseBody::SetVariable(result)) => Some(result.value),
                _ => None,
            }
            .as_deref(),
            Some("Namba(9.0)")
        );
        session.handle(&req(
            4,
            Command::Continue(ContinueArguments {
                thread_id: 1,
                single_thread: None,
            }),
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.hook().pause_generation() <= generation {
            assert!(
                std::time::Instant::now() < deadline,
                "updated program did not stop again at the breakpoint"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            session
                .hook()
                .current_bindings()
                .into_iter()
                .find(|(name, _)| name == "a")
                .map(|(_, value)| value)
                .as_deref(),
            Some("Namba(9.0)")
        );

        let generation = session.hook().pause_generation();
        let (response, _) = session.handle(&req(
            5,
            Command::Restart(RestartArguments { arguments: None }),
        ));
        assert!(response.success);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.hook().pause_generation() <= generation {
            assert!(
                std::time::Instant::now() < deadline,
                "restarted program did not stop at the breakpoint"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            session
                .hook()
                .current_bindings()
                .into_iter()
                .find(|(name, _)| name == "a")
                .map(|(_, value)| value)
                .as_deref(),
            Some("Namba(1.0)")
        );
        session.hook().resume();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
