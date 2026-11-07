//! `pata-dap`'s request/response handling: `initialize`, `launch`, `setBreakpoints`,
//! `configurationDone`, `continue`, `next`/`stepIn`/`stepOut`, `pause`, `stackTrace`, `scopes`,
//! `variables`, `setVariable`, `evaluate`, `restart`, `threads`, `disconnect`. `attach` is
//! answered with an error: an Asili program runs under the debugger only when the debugger
//! builds and starts it (a debug build marks every statement), so there is no running process
//! to attach to.
//!
//! Structurally mirrors `pata-lsp/src/server.rs`'s shape (a `Backend`-equivalent struct holding
//! shared state, a request/response loop) even though DAP and LSP are different protocols with
//! different message shapes — same overall "hold state, dispatch on message type" pattern.
//!
//! Requests answer from the session's state and the hook; what happens on the program's own
//! thread (it stopped, it finished) reaches the client as events through the session's event
//! sink (`events()`), which `run_session` connects to the wire.

use dap::events::{OutputEventBody, StoppedEventBody};
use dap::prelude::*;
use dap::types::{
    Breakpoint, Capabilities, OutputEventCategory, Scope, Source, StackFrame, StoppedEventReason,
    Thread, Variable,
};
use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::{Arc, Mutex};

use asili_evaluator::debug_hook::{DebugHook, DebugLiteral, StopReason};

/// Where events for the client go (the wire, once `run_session` connects it).
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// Starts and ends the program being debugged (`crate::runner::RealRunner` for a real `.as`
/// file). A session without one (the protocol tests against `MockHook`) answers requests only.
pub trait ProgramRunner: Send + Sync {
    /// Build and start `program` (from `launch`), sending its `exited`/`terminated` events
    /// (and an `output` event for a build error) to `events`.
    fn launch(&self, program: Option<String>, events: EventSink);
    /// End the running program, if any, without reporting it (`disconnect`, and before a
    /// `restart` launches again).
    fn stop(&self);
}

pub struct DapSession<H: DebugHook> {
    hook: Arc<H>,
    breakpoint_lines: Mutex<Vec<i64>>,
    source_path: Mutex<Option<String>>,
    /// The line last reported as paused-at, for `stackTrace` responses. `Arc`-wrapped so the
    /// program's thread (through the hook's stop listener, see `crate::runner`) can update it —
    /// `0` before any pause has occurred.
    paused_at_line: Arc<std::sync::atomic::AtomicI64>,
    runner: Option<Box<dyn ProgramRunner>>,
    sink: Arc<Mutex<Option<EventSink>>>,
}

impl<H: DebugHook> DapSession<H> {
    pub fn new(hook: Arc<H>) -> Self {
        Self {
            hook,
            breakpoint_lines: Mutex::new(Vec::new()),
            source_path: Mutex::new(None),
            paused_at_line: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            runner: None,
            sink: Arc::new(Mutex::new(None)),
        }
    }

    /// Start (on `configurationDone`), restart and stop the program through `runner`.
    pub fn with_runner(mut self, runner: impl ProgramRunner + 'static) -> Self {
        self.runner = Some(Box::new(runner));
        self
    }

    /// Send this session's events to `sink` (`run_session` connects the wire; tests collect).
    pub fn set_event_sink(&self, sink: EventSink) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    /// A sink forwarding to whatever `set_event_sink` connects, now or later — handed to the
    /// program's side before the wire exists. Events sent while nothing is connected are
    /// dropped.
    pub fn events(&self) -> EventSink {
        let sink = Arc::clone(&self.sink);
        Arc::new(move |event| {
            let current = sink.lock().unwrap().clone();
            if let Some(send) = current {
                send(event);
            }
        })
    }

    /// Record that execution is now paused at `line`, so a subsequent `stackTrace` request
    /// reports the real paused location rather than a stale or default one.
    pub fn set_paused_line(&self, line: i64) {
        self.paused_at_line
            .store(line, std::sync::atomic::Ordering::SeqCst);
    }

    /// Shared handle to this session's `paused_at_line`, for the program's side to update as it
    /// stops — see `crate::runner`.
    pub fn paused_at_line_handle(&self) -> Arc<std::sync::atomic::AtomicI64> {
        Arc::clone(&self.paused_at_line)
    }

    fn launch(&self) {
        if let Some(runner) = &self.runner {
            runner.launch(self.source_path.lock().unwrap().clone(), self.events());
        }
    }

    /// Handle one request, returning the `Response` to send back plus any `Event`s that should
    /// follow it (e.g. `initialize` is followed by an `initialized` event once the adapter is
    /// ready for `setBreakpoints`).
    pub fn handle(&self, req: &Request) -> (Response, Vec<Event>) {
        match &req.command {
            Command::Initialize(_) => {
                let capabilities = Capabilities {
                    supports_configuration_done_request: Some(true),
                    supports_set_variable: Some(true),
                    supports_evaluate_for_hovers: Some(true),
                    supports_restart_request: Some(true),
                    ..Default::default()
                };
                (
                    ok_response(req, ResponseBody::Initialize(capabilities)),
                    vec![Event::Initialized],
                )
            }
            Command::Launch(args) => {
                // `program` (the .as file to debug) arrives as an editor-specific launch.json
                // field, not a named LaunchRequestArguments field — DAP's own spec leaves this
                // to each adapter, carried in `additional_data`.
                if let Some(data) = &args.additional_data {
                    if let Some(program) = data.get("program").and_then(|v| v.as_str()) {
                        *self.source_path.lock().unwrap() = Some(program.to_string());
                    }
                }
                (ack(req), vec![])
            }
            Command::Attach(_) => (
                req.clone().error(
                    "kuunganisha (attach) hakuwezekani: programu ya Asili hutatuliwa ikianzishwa \
                     na kitatuzi chenyewe; tumia launch",
                ),
                vec![],
            ),
            Command::SetBreakpoints(args) => {
                let lines: Vec<i64> = args
                    .breakpoints
                    .as_ref()
                    .map(|bps| bps.iter().map(|b| b.line).collect())
                    .unwrap_or_default();
                *self.breakpoint_lines.lock().unwrap() = lines.clone();
                self.hook.set_breakpoints(
                    lines
                        .iter()
                        .filter_map(|&l| usize::try_from(l).ok())
                        .collect(),
                );

                let source_path = self.source_path.lock().unwrap().clone();
                let breakpoints: Vec<Breakpoint> = lines
                    .iter()
                    .map(|&line| Breakpoint {
                        verified: true,
                        line: Some(line),
                        source: Some(Source {
                            path: source_path.clone(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect();
                (
                    ok_response(
                        req,
                        ResponseBody::SetBreakpoints(responses::SetBreakpointsResponse {
                            breakpoints,
                        }),
                    ),
                    vec![],
                )
            }
            Command::ConfigurationDone => {
                self.launch();
                (ack(req), vec![])
            }
            Command::Threads => {
                let threads = vec![Thread {
                    id: 1,
                    name: "kuu".to_string(),
                }];
                (
                    ok_response(
                        req,
                        ResponseBody::Threads(responses::ThreadsResponse { threads }),
                    ),
                    vec![],
                )
            }
            Command::Continue(_) => {
                self.hook.resume();
                (
                    ok_response(
                        req,
                        ResponseBody::Continue(responses::ContinueResponse {
                            all_threads_continued: Some(true),
                        }),
                    ),
                    vec![],
                )
            }
            Command::Next(_) => {
                self.hook.step(asili_evaluator::debug_hook::StepKind::Over);
                (ack(req), vec![])
            }
            Command::StepIn(_) => {
                self.hook.step(asili_evaluator::debug_hook::StepKind::Into);
                (ack(req), vec![])
            }
            Command::StepOut(_) => {
                self.hook.step(asili_evaluator::debug_hook::StepKind::Out);
                (ack(req), vec![])
            }
            Command::Pause(_) => {
                self.hook.request_pause();
                (ack(req), vec![])
            }
            Command::Restart(_) => {
                if let Some(runner) = &self.runner {
                    runner.stop();
                }
                self.set_paused_line(0);
                self.launch();
                (ack(req), vec![])
            }
            Command::StackTrace(_) => {
                let source_path = self.source_path.lock().unwrap().clone();
                let line = self
                    .paused_at_line
                    .load(std::sync::atomic::Ordering::SeqCst);
                let stack_frames = vec![StackFrame {
                    id: 1,
                    name: "kuu".to_string(),
                    source: Some(Source {
                        path: source_path,
                        ..Default::default()
                    }),
                    line,
                    column: 0,
                    ..Default::default()
                }];
                (
                    ok_response(
                        req,
                        ResponseBody::StackTrace(responses::StackTraceResponse {
                            stack_frames,
                            total_frames: Some(1),
                        }),
                    ),
                    vec![],
                )
            }
            Command::Scopes(_) => {
                let scopes = vec![Scope {
                    name: "Locals".to_string(),
                    variables_reference: 1,
                    expensive: false,
                    ..Default::default()
                }];
                (
                    ok_response(
                        req,
                        ResponseBody::Scopes(responses::ScopesResponse { scopes }),
                    ),
                    vec![],
                )
            }
            Command::Variables(_) => {
                let variables: Vec<Variable> = self
                    .hook
                    .current_bindings()
                    .into_iter()
                    .map(|b| Variable {
                        name: b.name,
                        value: b.value,
                        variables_reference: 0,
                        ..Default::default()
                    })
                    .collect();
                (
                    ok_response(
                        req,
                        ResponseBody::Variables(responses::VariablesResponse { variables }),
                    ),
                    vec![],
                )
            }
            Command::SetVariable(args) => {
                let result = parse_literal(&args.value).and_then(|value| {
                    self.hook
                        .set_variable(&args.name, value.clone())
                        .map(|()| value)
                });
                match result {
                    Ok(value) => (
                        ok_response(
                            req,
                            ResponseBody::SetVariable(responses::SetVariableResponse {
                                value: value.display(),
                                type_field: Some(value.kind().name().to_string()),
                                ..Default::default()
                            }),
                        ),
                        vec![],
                    ),
                    Err(message) => (req.clone().error(&message), vec![]),
                }
            }
            Command::Evaluate(args) => match self.evaluate(&args.expression) {
                Ok(result) => (
                    ok_response(
                        req,
                        ResponseBody::Evaluate(responses::EvaluateResponse {
                            result,
                            variables_reference: 0,
                            ..Default::default()
                        }),
                    ),
                    vec![],
                ),
                Err(message) => (req.clone().error(&message), vec![]),
            },
            Command::Disconnect(_) => {
                if let Some(runner) = &self.runner {
                    runner.stop();
                }
                (ack(req), vec![Event::Terminated(None)])
            }
            other => (
                req.clone()
                    .error(&format!("amri haijatekelezwa: {}", command_name(other))),
                vec![],
            ),
        }
    }

    /// `evaluate`: the value of a name visible at the paused statement (what a hover or a watch
    /// asks for), or a literal's own value. Running other expressions would need compiling
    /// them into the paused program, which a native-only run cannot do mid-flight.
    fn evaluate(&self, expression: &str) -> Result<String, String> {
        let name = expression.trim();
        if let Some(b) = self
            .hook
            .current_bindings()
            .into_iter()
            .find(|b| b.name == name)
        {
            return Ok(b.value);
        }
        parse_literal(name).map(|l| l.display()).map_err(|_| {
            format!(
                "tathmini inakubali jina la kigeu kinachoonekana hapa au thamani halisi tu: {name}"
            )
        })
    }

    /// The breakpoint lines currently configured (last `setBreakpoints` call).
    pub fn breakpoint_lines(&self) -> Vec<i64> {
        self.breakpoint_lines.lock().unwrap().clone()
    }

    pub fn hook(&self) -> &Arc<H> {
        &self.hook
    }
}

/// A `stopped` event for the program pausing at `reason`.
pub fn stopped_event(reason: StopReason) -> Event {
    Event::Stopped(StoppedEventBody {
        reason: match reason {
            StopReason::Breakpoint => StoppedEventReason::Breakpoint,
            StopReason::Step => StoppedEventReason::Step,
            StopReason::Pause => StoppedEventReason::Pause,
        },
        description: None,
        thread_id: Some(1),
        preserve_focus_hint: None,
        text: None,
        all_threads_stopped: Some(true),
        hit_breakpoint_ids: None,
    })
}

/// An `output` event carrying an error for the user (e.g. the program failed to build).
pub fn error_output_event(message: String) -> Event {
    Event::Output(OutputEventBody {
        category: Some(OutputEventCategory::Stderr),
        output: format!("{message}\n"),
        ..Default::default()
    })
}

/// The value a `setVariable`/`evaluate` request writes as Asili source: a number (optionally
/// negative), a string literal, `kweli` or `si_kweli` — read by the language's own lexer, so
/// escapes in a string mean what they mean in a program.
pub fn parse_literal(text: &str) -> Result<DebugLiteral, String> {
    use asili_lexer::TokenKind;
    let invalid =
        || format!("thamani si sahihi: {text} (tumia namba, \"maandishi\", kweli au si_kweli)");
    let trimmed = text.trim();
    let (negative, body) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, trimmed),
    };
    let tokens = asili_lexer::tokenize(body).map_err(|_| invalid())?;
    let [token] = tokens.as_slice() else {
        return Err(invalid());
    };
    match (token.kind, negative) {
        (TokenKind::Number, _) => {
            let n: f64 = token
                .lexeme
                .replace('_', "")
                .parse()
                .map_err(|_| invalid())?;
            Ok(DebugLiteral::Namba(if negative { -n } else { n }))
        }
        (TokenKind::Str, false) => {
            let lexeme = &token.lexeme;
            Ok(DebugLiteral::Neno(lexeme[1..lexeme.len() - 1].to_string()))
        }
        (TokenKind::KwKweli, false) => Ok(DebugLiteral::Ukweli(true)),
        (TokenKind::KwSiKweli, false) => Ok(DebugLiteral::Ukweli(false)),
        _ => Err(invalid()),
    }
}

/// Build a successful response carrying `body` — thin wrapper over the `dap` crate's own
/// `Request::success` (which consumes `self`), so call sites can pass `req: &Request` without
/// needing to clone at every call site themselves.
fn ok_response(req: &Request, body: ResponseBody) -> Response {
    req.clone().success(body)
}

/// An acknowledgement-only response — thin wrapper over the `dap` crate's own `Request::ack()`,
/// which picks the correct empty/marker `ResponseBody` per command automatically (e.g.
/// `ResponseBody::Next`, `ResponseBody::ConfigurationDone`) rather than always `None`.
/// Falls back to a bodyless success response if `ack()` doesn't recognize the command.
fn ack(req: &Request) -> Response {
    req.clone().ack().unwrap_or_else(|_| Response {
        request_seq: req.seq,
        success: true,
        message: None,
        body: None,
        error: None,
    })
}

/// Bare command name for an unimplemented-command error message — debug-formatting the whole
/// `Command` (with its arguments) would be noisy there.
fn command_name(cmd: &Command) -> &'static str {
    match cmd {
        Command::Goto(_) => "goto",
        Command::GotoTargets(_) => "gotoTargets",
        Command::RestartFrame(_) => "restartFrame",
        Command::StepBack(_) => "stepBack",
        Command::ReverseContinue(_) => "reverseContinue",
        Command::SetExpression(_) => "setExpression",
        Command::Terminate(_) => "terminate",
        _ => "haijulikani",
    }
}

/// Run the DAP server loop over `input`/`output` (real stdio in `main.rs`, or an in-memory
/// buffer pair in tests) around a plain `DapSession` for `hook`, with nothing to launch.
pub fn run<H, R, W>(hook: Arc<H>, input: R, output: W)
where
    H: DebugHook,
    R: Read,
    W: Write + Send,
{
    run_session(Arc::new(DapSession::new(hook)), input, output);
}

/// Like `run`, but driven by an already-built `session` (e.g. `crate::runner::real_session`'s
/// output, with its runner attached), until the input stream closes or a `disconnect` request
/// is handled. Events the program's side sends meanwhile go out on the same wire, written by
/// a second thread while this one waits for the next request.
pub fn run_session<H, R, W>(session: Arc<DapSession<H>>, input: R, output: W)
where
    H: DebugHook,
    R: Read,
    W: Write + Send,
{
    let mut server = dap::server::Server::new(BufReader::new(input), BufWriter::new(output));
    let wire = Arc::clone(&server.output);
    let (send, events) = std::sync::mpsc::channel::<Event>();
    let send = Mutex::new(send);
    session.set_event_sink(Arc::new(move |event| {
        let _ = send.lock().unwrap().send(event);
    }));

    std::thread::scope(|scope| {
        scope.spawn(|| {
            for event in events {
                if wire.lock().unwrap().send_event(event).is_err() {
                    break;
                }
            }
        });
        loop {
            match server.poll_request() {
                Ok(Some(req)) => {
                    let is_disconnect = matches!(req.command, Command::Disconnect(_));
                    let (response, events) = session.handle(&req);
                    if server.respond(response).is_err() {
                        break;
                    }
                    for event in events {
                        if server.send_event(event).is_err() {
                            break;
                        }
                    }
                    if is_disconnect {
                        break;
                    }
                }
                Ok(None) => break, // input closed
                Err(_) => break,
            }
        }
        // Dropping the sink closes the channel, ending the writer thread.
        *session.sink.lock().unwrap() = None;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_hook::MockHook;
    use dap::requests::{
        ContinueArguments, EvaluateArguments, InitializeArguments, LaunchRequestArguments,
        NextArguments, PauseArguments, ScopesArguments, SetBreakpointsArguments,
        SetVariableArguments, StackTraceArguments, StepInArguments, StepOutArguments,
        VariablesArguments,
    };
    use dap::types::SourceBreakpoint;

    fn session() -> DapSession<MockHook> {
        DapSession::new(Arc::new(MockHook::new()))
    }

    fn req(seq: i64, command: Command) -> Request {
        Request { seq, command }
    }

    #[test]
    fn initialize_responds_with_capabilities_and_an_initialized_event() {
        let s = session();
        let (resp, events) = s.handle(&req(1, Command::Initialize(InitializeArguments::default())));
        assert!(resp.success);
        assert!(matches!(resp.body, Some(ResponseBody::Initialize(_))));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Initialized));
    }

    #[test]
    fn launch_acknowledges_and_stores_the_program_path() {
        let s = session();
        let args = LaunchRequestArguments {
            no_debug: None,
            restart_data: None,
            additional_data: Some(serde_json::json!({"program": "/tmp/mfano.as"})),
        };
        let (resp, _) = s.handle(&req(2, Command::Launch(args)));
        assert!(resp.success);

        // The stored path surfaces in a later stackTrace response's Source.
        let (st_resp, _) = s.handle(&req(
            3,
            Command::StackTrace(StackTraceArguments {
                thread_id: 1,
                start_frame: None,
                levels: None,
                format: None,
            }),
        ));
        let ResponseBody::StackTrace(body) = st_resp.body.unwrap() else {
            panic!("expected StackTrace body")
        };
        assert_eq!(
            body.stack_frames[0]
                .source
                .as_ref()
                .unwrap()
                .path
                .as_deref(),
            Some("/tmp/mfano.as")
        );
    }

    #[test]
    fn set_breakpoints_returns_one_verified_breakpoint_per_line() {
        let s = session();
        let args = SetBreakpointsArguments {
            source: dap::types::Source::default(),
            breakpoints: Some(vec![
                SourceBreakpoint {
                    line: 5,
                    ..Default::default()
                },
                SourceBreakpoint {
                    line: 9,
                    ..Default::default()
                },
            ]),
            #[allow(deprecated)]
            lines: None,
            source_modified: None,
        };
        let (resp, _) = s.handle(&req(4, Command::SetBreakpoints(args)));
        assert!(resp.success);
        let ResponseBody::SetBreakpoints(body) = resp.body.unwrap() else {
            panic!("expected SetBreakpoints body")
        };
        assert_eq!(body.breakpoints.len(), 2);
        assert!(body.breakpoints.iter().all(|b| b.verified));
        assert_eq!(body.breakpoints[0].line, Some(5));
        assert_eq!(body.breakpoints[1].line, Some(9));

        assert_eq!(s.breakpoint_lines(), vec![5, 9]);
    }

    #[test]
    fn continue_resumes_the_hook() {
        let hook = Arc::new(MockHook::new());
        let s = DapSession::new(Arc::clone(&hook));

        // Pause the hook on a background thread, then continue via the session and confirm the
        // pause actually released -- a real behavioral check, not just "the call didn't error."
        hook.set_breakpoints(vec![3]);
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause(3, 1));

        for _ in 0..100 {
            if hook.did_pause() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(hook.did_pause());

        let (resp, _) = s.handle(&req(
            5,
            Command::Continue(ContinueArguments {
                thread_id: 1,
                single_thread: None,
            }),
        ));
        assert!(resp.success);
        assert!(
            pause_thread.join().unwrap(),
            "continue must have released the paused thread"
        );
    }

    #[test]
    fn stack_trace_reports_the_last_paused_line() {
        let s = session();
        s.set_paused_line(42);
        let (resp, _) = s.handle(&req(
            6,
            Command::StackTrace(StackTraceArguments {
                thread_id: 1,
                start_frame: None,
                levels: None,
                format: None,
            }),
        ));
        let ResponseBody::StackTrace(body) = resp.body.unwrap() else {
            panic!("expected StackTrace body")
        };
        assert_eq!(body.stack_frames[0].line, 42);
    }

    #[test]
    fn scopes_returns_a_locals_scope() {
        let s = session();
        let (resp, _) = s.handle(&req(7, Command::Scopes(ScopesArguments { frame_id: 1 })));
        let ResponseBody::Scopes(body) = resp.body.unwrap() else {
            panic!("expected Scopes body")
        };
        assert_eq!(body.scopes.len(), 1);
        assert_eq!(body.scopes[0].name, "Locals");
    }

    #[test]
    fn variables_reflects_the_hooks_real_bindings() {
        let hook = Arc::new(MockHook::new());
        hook.set_bindings(vec![("jumla".to_string(), "10".to_string())]);
        let s = DapSession::new(hook);

        let (resp, _) = s.handle(&req(
            8,
            Command::Variables(VariablesArguments {
                variables_reference: 1,
                filter: None,
                start: None,
                count: None,
                format: None,
            }),
        ));
        let ResponseBody::Variables(body) = resp.body.unwrap() else {
            panic!("expected Variables body")
        };
        assert_eq!(body.variables.len(), 1);
        assert_eq!(body.variables[0].name, "jumla");
        assert_eq!(body.variables[0].value, "10");
    }

    #[test]
    fn disconnect_acknowledges_and_sends_a_terminated_event() {
        let s = session();
        let (resp, events) = s.handle(&req(9, Command::Disconnect(Default::default())));
        assert!(resp.success);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Terminated(_)));
    }

    #[test]
    fn attach_is_refused_with_a_reason() {
        let s = session();
        let (resp, _) = s.handle(&req(10, Command::Attach(Default::default())));
        assert!(!resp.success, "attach must fail, not silently succeed");
        assert!(matches!(
            resp.message,
            Some(responses::ResponseMessage::Error(_))
        ));
    }

    #[test]
    fn an_unimplemented_command_returns_a_real_error_response() {
        let s = session();
        let (resp, _) = s.handle(&req(10, Command::Terminate(Default::default())));
        assert!(!resp.success);
    }

    #[test]
    fn initialize_advertises_set_variable_evaluate_and_restart() {
        let s = session();
        let (resp, _) = s.handle(&req(1, Command::Initialize(InitializeArguments::default())));
        let Some(ResponseBody::Initialize(caps)) = resp.body else {
            panic!("expected Initialize body")
        };
        assert_eq!(caps.supports_set_variable, Some(true));
        assert_eq!(caps.supports_evaluate_for_hovers, Some(true));
        assert_eq!(caps.supports_restart_request, Some(true));
    }

    /// Pause the hook at `line` on a background thread (depth 1) and wait until it blocks.
    fn paused_at(hook: &Arc<MockHook>, line: usize) -> std::thread::JoinHandle<bool> {
        hook.set_breakpoints(vec![line]);
        let h = Arc::clone(hook);
        let t = std::thread::spawn(move || h.should_pause(line, 1));
        for _ in 0..400 {
            if hook.paused_at_line() == Some(line) {
                return t;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("never paused at {line}");
    }

    /// next/stepIn/stepOut each release the paused program (the hook decides where it stops
    /// next — see `debug_hook`'s own tests and the evaluator's end-to-end ones).
    #[test]
    fn step_requests_release_the_paused_program() {
        let hook = Arc::new(MockHook::new());
        let s = DapSession::new(Arc::clone(&hook));
        let commands = [
            Command::Next(NextArguments {
                thread_id: 1,
                single_thread: None,
                granularity: None,
            }),
            Command::StepIn(StepInArguments {
                thread_id: 1,
                single_thread: None,
                target_id: None,
                granularity: None,
            }),
            Command::StepOut(StepOutArguments {
                thread_id: 1,
                single_thread: None,
                granularity: None,
            }),
        ];
        for (i, command) in commands.into_iter().enumerate() {
            let t = paused_at(&hook, 3);
            let (resp, _) = s.handle(&req(20 + i as i64, command));
            assert!(resp.success);
            assert!(t.join().unwrap(), "the step must have released the program");
            hook.set_breakpoints(vec![]);
        }
    }

    #[test]
    fn pause_stops_the_program_at_its_next_statement() {
        let hook = Arc::new(MockHook::new());
        let s = DapSession::new(Arc::clone(&hook));
        let (resp, _) = s.handle(&req(30, Command::Pause(PauseArguments { thread_id: 1 })));
        assert!(resp.success);
        let h = Arc::clone(&hook);
        let t = std::thread::spawn(move || h.should_pause(8, 1));
        for _ in 0..400 {
            if hook.paused_at_line() == Some(8) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(hook.paused_at_line(), Some(8));
        hook.resume();
        assert!(t.join().unwrap());
    }

    fn set_variable(s: &DapSession<MockHook>, name: &str, value: &str) -> Response {
        s.handle(&req(
            40,
            Command::SetVariable(SetVariableArguments {
                variables_reference: 1,
                name: name.to_string(),
                value: value.to_string(),
                format: None,
            }),
        ))
        .0
    }

    #[test]
    fn set_variable_writes_a_literal_of_the_variables_kind() {
        let s = session();
        let resp = set_variable(&s, "x", "-7.5");
        assert!(resp.success, "{resp:?}");
        let Some(ResponseBody::SetVariable(body)) = resp.body else {
            panic!("expected SetVariable body")
        };
        assert_eq!(body.type_field.as_deref(), Some("Namba"));
        assert!(s.hook().current_bindings()[0].value.contains("-7.5"));

        assert!(set_variable(&s, "jina", "\"habari\\n\"").success);
        assert!(!set_variable(&s, "x", "\"si namba\"").success, "wrong kind");
        assert!(!set_variable(&s, "x", "1 + 2").success, "not a literal");
        assert!(!set_variable(&s, "hakuna", "1").success, "no such variable");
    }

    fn evaluate(s: &DapSession<MockHook>, expression: &str) -> Response {
        s.handle(&req(
            50,
            Command::Evaluate(EvaluateArguments {
                expression: expression.to_string(),
                frame_id: None,
                context: None,
                format: None,
            }),
        ))
        .0
    }

    #[test]
    fn evaluate_reads_a_visible_variable_or_a_literal() {
        let s = session();
        let Some(ResponseBody::Evaluate(body)) = evaluate(&s, " x ").body else {
            panic!("expected Evaluate body")
        };
        assert_eq!(body.result, "42");
        let Some(ResponseBody::Evaluate(body)) = evaluate(&s, "si_kweli").body else {
            panic!("expected Evaluate body")
        };
        assert!(
            body.result.contains("false") || body.result.contains("kweli"),
            "{}",
            body.result
        );
        assert!(!evaluate(&s, "x + 1").success);
    }

    #[test]
    fn parse_literal_reads_asili_literals() {
        assert_eq!(parse_literal("12"), Ok(DebugLiteral::Namba(12.0)));
        assert_eq!(parse_literal(" -0.5 "), Ok(DebugLiteral::Namba(-0.5)));
        assert_eq!(parse_literal("kweli"), Ok(DebugLiteral::Ukweli(true)));
        assert_eq!(parse_literal("si_kweli"), Ok(DebugLiteral::Ukweli(false)));
        assert_eq!(
            parse_literal("\"a\\tb\""),
            Ok(DebugLiteral::Neno("a\tb".to_string()))
        );
        assert!(parse_literal("-kweli").is_err());
        assert!(parse_literal("x").is_err());
        assert!(parse_literal("").is_err());
    }

    /// Records what the session asks of its runner.
    #[derive(Clone, Default)]
    struct FakeRunner(Arc<Mutex<Vec<String>>>);

    impl ProgramRunner for FakeRunner {
        fn launch(&self, program: Option<String>, events: EventSink) {
            self.0
                .lock()
                .unwrap()
                .push(format!("launch {}", program.unwrap_or_default()));
            events(Event::Terminated(None));
        }
        fn stop(&self) {
            self.0.lock().unwrap().push("stop".to_string());
        }
    }

    #[test]
    fn configuration_done_restart_and_disconnect_drive_the_runner() {
        let runner = FakeRunner::default();
        let s = session().with_runner(runner.clone());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_by_sink = Arc::clone(&seen);
        s.set_event_sink(Arc::new(move |e| seen_by_sink.lock().unwrap().push(e)));

        let args = LaunchRequestArguments {
            no_debug: None,
            restart_data: None,
            additional_data: Some(serde_json::json!({"program": "kuu.as"})),
        };
        s.handle(&req(1, Command::Launch(args)));
        s.handle(&req(2, Command::ConfigurationDone));
        let (resp, _) = s.handle(&req(
            3,
            Command::Restart(dap::requests::RestartArguments { arguments: None }),
        ));
        assert!(resp.success);
        s.handle(&req(4, Command::Disconnect(Default::default())));

        assert_eq!(
            *runner.0.lock().unwrap(),
            vec!["launch kuu.as", "stop", "launch kuu.as", "stop"]
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "the runner's events reach the sink"
        );
    }

    /// Full end-to-end: real DAP wire-protocol bytes (`Content-Length: N\r\n\r\n<json>`) in,
    /// real wire-protocol bytes out — through the actual `run()` stdio loop, not just
    /// `DapSession::handle` in isolation. Matches the `dap` crate's own `server.rs` test pattern.
    #[test]
    fn run_handles_a_real_wire_protocol_request_end_to_end() {
        let request_json =
            r#"{"seq":1,"type":"request","command":"initialize","arguments":{"adapterID":"test"}}"#;
        let input = format!(
            "Content-Length: {}\r\n\r\n{}",
            request_json.len(),
            request_json
        );

        let mut output = Vec::new();
        let hook = Arc::new(MockHook::new());
        run(hook, std::io::Cursor::new(input.into_bytes()), &mut output);

        let output_str = String::from_utf8(output).expect("valid utf8 output");
        assert!(
            output_str.contains("Content-Length:"),
            "output should be real wire-protocol framed, got: {output_str}"
        );
        assert!(output_str.contains("\"success\":true"), "got: {output_str}");
    }
}
