//! Wires a real `RealDebugHook` up to an actual running `.as` program: compiles the file named
//! by a DAP `launch` request's `program` field and runs it on a background thread once
//! `configurationDone` fires. The hook's stop listener keeps `DapSession::paused_at_line`
//! current and sends the client a `stopped` event each time the program pauses; the run's end
//! sends `exited` and `terminated`. `restart` stops the run (the host ends it at its next
//! statement), waits for it, and launches again; `disconnect` stops it.
//!
//! Single-file compilation only (lex → parse — no project/dependency resolution, no semantic
//! check) — matches the DAP use case of "debug this one file," not a full `pata jenga` project
//! build. A project-aware `launch` (resolving `leta` imports via `pata-core`) is a reasonable
//! future extension.

use asili_evaluator::debug_hook::RealDebugHook;
use asili_lexer::tokenize;
use asili_parser::parse_tokens;
use dap::events::ExitedEventBody;
use dap::prelude::Event;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::server::{error_output_event, stopped_event, DapSession, EventSink, ProgramRunner};

/// Build a `DapSession<RealDebugHook>` that compiles and runs whatever `.as` file a `launch`
/// request names, once `configurationDone` arrives, reporting each pause as a `stopped` event.
pub fn real_session() -> Arc<DapSession<RealDebugHook>> {
    let hook = Arc::new(RealDebugHook::new(Vec::new()));
    let session = DapSession::new(Arc::clone(&hook));
    let paused_at_line = session.paused_at_line_handle();
    let events = session.events();
    hook.set_stop_listener(Box::new(move |reason, line| {
        paused_at_line.store(line as i64, Ordering::SeqCst);
        events(stopped_event(reason));
    }));
    let runner = RealRunner {
        hook,
        run: Mutex::new(None),
    };
    Arc::new(session.with_runner(runner))
}

/// The current run: its thread, and whether its end should go unreported (it was stopped).
struct Run {
    thread: JoinHandle<()>,
    silenced: Arc<AtomicBool>,
}

pub struct RealRunner {
    hook: Arc<RealDebugHook>,
    run: Mutex<Option<Run>>,
}

impl ProgramRunner for RealRunner {
    fn launch(&self, program: Option<String>, events: EventSink) {
        let module = match compile(program) {
            Ok(module) => module,
            Err(message) => {
                events(error_output_event(message));
                events(Event::Terminated(None));
                return;
            }
        };
        self.hook.reset();
        let silenced = Arc::new(AtomicBool::new(false));
        let hook = Arc::clone(&self.hook);
        let quiet = Arc::clone(&silenced);
        let thread = std::thread::spawn(move || {
            let ok = asili_evaluator::run_main_with_debug_hook(&module, Vec::new(), hook).is_ok();
            if !quiet.load(Ordering::SeqCst) {
                events(Event::Exited(ExitedEventBody {
                    exit_code: if ok { 0 } else { 1 },
                }));
                events(Event::Terminated(None));
            }
        });
        *self.run.lock().unwrap() = Some(Run { thread, silenced });
    }

    fn stop(&self) {
        let Some(run) = self.run.lock().unwrap().take() else {
            return;
        };
        run.silenced.store(true, Ordering::SeqCst);
        self.hook.set_stop(true);
        let _ = run.thread.join();
        self.hook.reset();
    }
}

/// Read and parse the file a `launch` named, or say (in Swahili, for the client's console) why
/// it can't be debugged.
fn compile(program: Option<String>) -> Result<asili_parser::Module, String> {
    let path = program.ok_or("launch haikutaja programu (\"program\") ya kutatua")?;
    let source =
        std::fs::read_to_string(&path).map_err(|e| format!("imeshindwa kusoma {path}: {e}"))?;
    let tokens = tokenize(&source).map_err(|d| format!("{path}: makosa {} ya leksika", d.len()))?;
    parse_tokens(&tokens).map_err(|d| format!("{path}: makosa {} ya uchanganuzi", d.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asili_evaluator::debug_hook::DebugHook;
    use dap::prelude::*;
    use dap::requests::{LaunchRequestArguments, SetBreakpointsArguments};
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
            "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka a = 1\n  weka b = 2\n}",
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
        bp_args.breakpoints = Some(vec![SourceBreakpoint {
            line: 3,
            ..Default::default()
        }]);
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

        session.hook().resume();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// What the client sees, in order: events sent by the program's side.
    fn collect_events(session: &DapSession<RealDebugHook>) -> Arc<Mutex<Vec<Event>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        session.set_event_sink(Arc::new(move |e| sink.lock().unwrap().push(e)));
        seen
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn stops(seen: &Mutex<Vec<Event>>) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Event::Stopped(_)))
            .count()
    }

    /// A whole session against a real program: a breakpoint stops it (`stopped` event), `next`
    /// stops it on the following line, `restart` runs it again from the top (stopping at the
    /// breakpoint once more, the old run unreported), and letting it finish sends `exited` and
    /// `terminated`.
    #[test]
    fn a_real_session_stops_steps_restarts_and_ends() {
        let path = temp_as_file(
            "session",
            "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n  weka a = 1\n  weka b = a + 1\n  weka c = b\n}",
        );
        let session = real_session();
        let seen = collect_events(&session);

        let mut launch_args = LaunchRequestArguments::default();
        launch_args.additional_data = Some(serde_json::json!({"program": path.to_string_lossy()}));
        session.handle(&req(1, Command::Launch(launch_args)));
        let mut bp_args = SetBreakpointsArguments::default();
        bp_args.breakpoints = Some(vec![SourceBreakpoint {
            line: 2,
            ..Default::default()
        }]);
        session.handle(&req(2, Command::SetBreakpoints(bp_args)));
        session.handle(&req(3, Command::ConfigurationDone));

        wait_for("the breakpoint", || stops(&seen) == 1);
        assert_eq!(session.hook().paused_at_line(), Some(2));
        session.handle(&req(4, Command::Next(Default::default())));
        wait_for("the step", || stops(&seen) == 2);
        assert_eq!(session.hook().paused_at_line(), Some(3));

        session.handle(&req(
            5,
            Command::Restart(dap::requests::RestartArguments { arguments: None }),
        ));
        wait_for("the breakpoint after restart", || stops(&seen) == 3);
        assert_eq!(session.hook().paused_at_line(), Some(2));
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, Event::Terminated(_))),
            "the stopped run must not report terminating"
        );

        session.handle(&req(6, Command::Continue(Default::default())));
        wait_for("the end of the run", || {
            seen.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, Event::Terminated(_)))
        });
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Exited(ExitedEventBody { exit_code: 0 }))));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn launching_a_missing_file_reports_why_and_terminates() {
        let session = real_session();
        let seen = collect_events(&session);
        let mut launch_args = LaunchRequestArguments::default();
        launch_args.additional_data = Some(serde_json::json!({"program": "/hakuna/kuu.as"}));
        session.handle(&req(1, Command::Launch(launch_args)));
        session.handle(&req(2, Command::ConfigurationDone));
        let seen = seen.lock().unwrap();
        assert!(matches!(seen[0], Event::Output(_)));
        assert!(matches!(seen[1], Event::Terminated(_)));
    }
}
