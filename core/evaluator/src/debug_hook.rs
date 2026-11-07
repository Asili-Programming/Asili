//! Real step-through debugging support: the `DebugHook` trait `pata-dap` drives a running
//! program through, plus `RealDebugHook`, the actual (not test-only) implementation the
//! native code's host calls into at every statement of a debug build (`Opcode::Line`).
//!
//! Lives here (not in `pata-dap`) because `core/evaluator` must be able to implement this trait
//! without depending on `pata/dap` — this project's crates never have a `core/` crate depend on
//! a `pata/` one (`pata-dap` depends on `asili-evaluator`, the normal direction, matching
//! `pata-cli`/`pata-runner`'s existing precedent). `pata-dap` re-exports `DebugHook` from here
//! rather than defining its own copy, so both sides of the contract stay in sync by construction.
//!
//! What a hook decides at each statement (`should_pause`): stop at a breakpoint line, at the
//! next statement after a `pause` request, or where a step ends — a step is measured in call
//! depth (debug builds inline no call, so the host's depth is exact at every statement): *into*
//! stops at the very next statement, *over* at the next one no deeper than where it started,
//! *out* at the next one shallower than that. After the pause the host applies any variable
//! writes the debugger queued (`take_writes`), then ends the run if the debugger asked it to
//! (`should_stop`: `restart`/`disconnect`).

// Runtime code never panics on its own: an impossible state is an error the program sees
// (and its safe state handles), not a crash (see docs/design/safety-critical-roadmap.md §3).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::value::Value;

/// The values a debugger can write into a paused program's variable: the scalar kinds, which
/// carry no references and so cross from the debugger's thread to the program's.
#[derive(Clone, Debug, PartialEq)]
pub enum DebugLiteral {
    Namba(f64),
    Ukweli(bool),
    Neno(String),
}

impl DebugLiteral {
    pub fn kind(&self) -> LiteralKind {
        match self {
            DebugLiteral::Namba(_) => LiteralKind::Namba,
            DebugLiteral::Ukweli(_) => LiteralKind::Ukweli,
            DebugLiteral::Neno(_) => LiteralKind::Neno,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            DebugLiteral::Namba(n) => Value::Namba(*n),
            DebugLiteral::Ukweli(b) => Value::Ukweli(*b),
            DebugLiteral::Neno(s) => Value::neno(s.as_str()),
        }
    }

    /// How the variables view shows this value (the same text the host records for it).
    pub fn display(&self) -> String {
        format!("{:?}", self.to_value())
    }
}

/// The type of a variable a debugger may overwrite (`None` on a [`Binding`]: not writable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralKind {
    Namba,
    Ukweli,
    Neno,
}

impl LiteralKind {
    pub fn of(value: &Value) -> Option<LiteralKind> {
        match value {
            Value::Namba(_) => Some(LiteralKind::Namba),
            Value::Ukweli(_) => Some(LiteralKind::Ukweli),
            Value::Neno(_) => Some(LiteralKind::Neno),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LiteralKind::Namba => "Namba",
            LiteralKind::Ukweli => "Ukweli",
            LiteralKind::Neno => "Neno",
        }
    }
}

/// One name visible at the paused statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub name: String,
    /// The value as the variables view shows it.
    pub value: String,
    /// The kind a write must have, when the debugger may write this binding: a local holding a
    /// `Namba`, `Ukweli` or `Neno` (predefined names and other values are read-only).
    pub writable: Option<LiteralKind>,
}

/// How far a step runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// To the very next statement, entering a call.
    Into,
    /// To the next statement of this function (or its caller), running calls through.
    Over,
    /// To the next statement of the caller.
    Out,
}

/// Why the program last stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint,
    Step,
    Pause,
}

/// A running program's debug-control surface — implemented by `RealDebugHook` below (wired into
/// native code's host by `run_main_with_debug_hook`) and, in `pata-dap`'s own test suite, by a
/// canned test double exercising the DAP protocol layer independent of a real running program.
pub trait DebugHook: Send + Sync {
    /// Called by native code's host immediately before `should_pause`, with a fresh snapshot of
    /// every currently-in-scope binding (innermost-scope-wins on a shadowed name) — so a real
    /// pause always has up-to-date data ready for a `variables` request before it can possibly
    /// block. Takes an already-formatted snapshot rather than live registers, keeping
    /// this trait's shape independent of the host's frames.
    fn record_bindings(&self, bindings: Vec<Binding>);
    /// Called before executing the statement at `line`, `depth` calls deep. Returning `true`
    /// means execution genuinely paused and has since resumed — a real implementation blocks
    /// the calling thread internally (on a condvar wait) until `resume()`/`step()` is called
    /// from another thread, rather than the caller polling in a loop.
    fn should_pause(&self, line: usize, depth: u64) -> bool;
    fn resume(&self);
    /// Snapshot of currently-in-scope bindings, for a `variables` DAP request — whatever was
    /// last passed to `record_bindings`, with any writes made since applied.
    fn current_bindings(&self) -> Vec<Binding>;
    /// Resume, stopping again where a `kind` step from the paused statement ends.
    fn step(&self, kind: StepKind);
    /// Stop at the next statement the program reaches.
    fn request_pause(&self);
    /// While paused: overwrite the local `name` with `value` once the program resumes. Fails
    /// (with a message for the user) when no writable local has that name or `value` is of a
    /// different kind than the local holds.
    fn set_variable(&self, name: &str, value: DebugLiteral) -> Result<(), String>;
    /// The writes queued by `set_variable` during the pause just ended, for the host to apply.
    fn take_writes(&self) -> Vec<(String, DebugLiteral)>;
    /// Whether the run should end now (the debugger restarts or disconnects).
    fn should_stop(&self) -> bool;
    /// The lines that stop the program (a `setBreakpoints` request; applies to a running
    /// program from its next statement).
    fn set_breakpoints(&self, lines: Vec<usize>);
}

/// What the program is told to do at its next statements.
#[derive(Default)]
struct Control {
    /// Paused right now (the program thread waits while this holds).
    paused: bool,
    /// A step in progress: its kind and the depth it started at.
    step: Option<(StepKind, u64)>,
    /// The depth of the statement the program is paused at.
    depth: u64,
}

/// Called with where and why the program stopped, from the program's thread, right before it
/// blocks — how `pata-dap` learns to send a `stopped` event.
pub type StopListener = Box<dyn Fn(StopReason, usize) + Send + Sync>;

/// The real, evaluator-side `DebugHook`: pauses at configured breakpoint lines, after a pause
/// request, or where a step ends, blocking the executing thread on a `Mutex` + `Condvar` until
/// `resume()`/`step()` is called from the DAP server's own request-handling thread.
/// `record_bindings` is called by the host right before `should_pause`, so a `variables`
/// request issued while genuinely paused reflects the real, live environment at the paused
/// line — not canned data.
pub struct RealDebugHook {
    breakpoints: Mutex<Vec<usize>>,
    control: Arc<(Mutex<Control>, Condvar)>,
    bindings: Mutex<Vec<Binding>>,
    writes: Mutex<Vec<(String, DebugLiteral)>>,
    pause_requested: AtomicBool,
    stop: AtomicBool,
    did_pause: AtomicBool,
    /// The line currently paused at, or `-1` when not paused.
    paused_at_line: AtomicI64,
    last_reason: Mutex<Option<StopReason>>,
    on_stop: Mutex<Option<StopListener>>,
}

impl RealDebugHook {
    pub fn new(breakpoints: Vec<usize>) -> Self {
        Self {
            breakpoints: Mutex::new(breakpoints),
            control: Arc::new((Mutex::new(Control::default()), Condvar::new())),
            bindings: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            pause_requested: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            did_pause: AtomicBool::new(false),
            paused_at_line: AtomicI64::new(-1),
            last_reason: Mutex::new(None),
            on_stop: Mutex::new(None),
        }
    }

    pub fn set_breakpoints(&self, lines: Vec<usize>) {
        *crate::sync::lock(&self.breakpoints) = lines;
    }

    /// Call `listener` every time the program stops (see [`StopListener`]).
    pub fn set_stop_listener(&self, listener: StopListener) {
        *crate::sync::lock(&self.on_stop) = Some(listener);
    }

    /// Whether execution has genuinely paused at least once.
    pub fn did_pause(&self) -> bool {
        self.did_pause.load(Ordering::SeqCst)
    }

    /// The line currently paused at, or `None` when execution isn't paused right now.
    pub fn paused_at_line(&self) -> Option<usize> {
        let line = self.paused_at_line.load(Ordering::SeqCst);
        if line < 0 {
            None
        } else {
            Some(line as usize)
        }
    }

    /// Why the program last stopped, if it has.
    pub fn last_stop_reason(&self) -> Option<StopReason> {
        *crate::sync::lock(&self.last_reason)
    }

    /// End the current run at its next statement (releasing it if paused), or — `false` —
    /// let the next run go on normally (`restart` clears it before launching again).
    pub fn set_stop(&self, stop: bool) {
        self.stop.store(stop, Ordering::SeqCst);
        if stop {
            self.release(None);
        }
    }

    /// Return the hook to how a new run starts: not paused, no step, no pause request, no
    /// queued writes, no stop (breakpoints and the stop listener are kept).
    pub fn reset(&self) {
        self.stop.store(false, Ordering::SeqCst);
        self.pause_requested.store(false, Ordering::SeqCst);
        crate::sync::lock(&self.writes).clear();
        crate::sync::lock(&self.bindings).clear();
        let (lock, _) = &*self.control;
        *crate::sync::lock(lock) = Control::default();
        self.paused_at_line.store(-1, Ordering::SeqCst);
    }

    /// Unblock the paused program, starting `step` (if any) from the depth it is paused at.
    fn release(&self, step: Option<StepKind>) {
        let (lock, cvar) = &*self.control;
        let mut control = crate::sync::lock(lock);
        control.step = step.map(|kind| (kind, control.depth));
        control.paused = false;
        cvar.notify_all();
    }

    /// Why the statement at `line`, `depth` deep, should stop the program, if it should.
    fn stop_reason(&self, line: usize, depth: u64) -> Option<StopReason> {
        if self.pause_requested.swap(false, Ordering::SeqCst) {
            return Some(StopReason::Pause);
        }
        let (lock, _) = &*self.control;
        let step_done = match crate::sync::lock(lock).step {
            Some((StepKind::Into, _)) => true,
            Some((StepKind::Over, from)) => depth <= from,
            Some((StepKind::Out, from)) => depth < from,
            None => false,
        };
        if step_done {
            return Some(StopReason::Step);
        }
        if crate::sync::lock(&self.breakpoints).contains(&line) {
            return Some(StopReason::Breakpoint);
        }
        None
    }
}

impl Default for RealDebugHook {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl DebugHook for RealDebugHook {
    fn record_bindings(&self, bindings: Vec<Binding>) {
        *crate::sync::lock(&self.bindings) = bindings;
    }

    fn should_pause(&self, line: usize, depth: u64) -> bool {
        if self.should_stop() {
            return false;
        }
        let Some(reason) = self.stop_reason(line, depth) else {
            return false;
        };

        self.did_pause.store(true, Ordering::SeqCst);
        *crate::sync::lock(&self.last_reason) = Some(reason);
        self.paused_at_line.store(line as i64, Ordering::SeqCst);
        let (lock, cvar) = &*self.control;
        {
            let mut control = crate::sync::lock(lock);
            control.paused = true;
            control.step = None;
            control.depth = depth;
        }
        // Told after `paused` is set, so a `continue`/step sent in reply to the `stopped` event
        // finds the program paused and releases it.
        if let Some(listener) = &*crate::sync::lock(&self.on_stop) {
            listener(reason, line);
        }
        let mut control = crate::sync::lock(lock);
        while control.paused {
            control = cvar
                .wait(control)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        self.paused_at_line.store(-1, Ordering::SeqCst);
        true
    }

    fn resume(&self) {
        self.release(None);
    }

    fn current_bindings(&self) -> Vec<Binding> {
        crate::sync::lock(&self.bindings).clone()
    }

    fn step(&self, kind: StepKind) {
        self.release(Some(kind));
    }

    fn request_pause(&self) {
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    fn set_variable(&self, name: &str, value: DebugLiteral) -> Result<(), String> {
        if self.paused_at_line().is_none() {
            return Err("programu haijasimama; kigeu kinabadilishwa ikiwa imesimama tu".into());
        }
        write_binding(&mut crate::sync::lock(&self.bindings), name, &value)?;
        crate::sync::lock(&self.writes).push((name.to_string(), value));
        Ok(())
    }

    fn take_writes(&self) -> Vec<(String, DebugLiteral)> {
        std::mem::take(&mut *crate::sync::lock(&self.writes))
    }

    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn set_breakpoints(&self, lines: Vec<usize>) {
        RealDebugHook::set_breakpoints(self, lines);
    }
}

/// Check that `value` may be written to the binding `name` and update its shown value — the
/// one rule for what a debugger may write, shared by every hook.
pub fn write_binding(
    bindings: &mut [Binding],
    name: &str,
    value: &DebugLiteral,
) -> Result<(), String> {
    let Some(binding) = bindings.iter_mut().find(|b| b.name == name) else {
        return Err(format!("hakuna kigeu kinachoitwa '{name}' hapa"));
    };
    let Some(kind) = binding.writable else {
        return Err(format!("kigeu '{name}' hakiwezi kubadilishwa na kitatuzi"));
    };
    if kind != value.kind() {
        return Err(format!(
            "kigeu '{name}' ni {}, si {}",
            kind.name(),
            value.kind().name()
        ));
    }
    binding.value = value.display();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait_paused(hook: &RealDebugHook) {
        for _ in 0..400 {
            if hook.paused_at_line().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the program never paused");
    }

    #[test]
    fn should_pause_returns_false_for_a_line_with_no_breakpoint() {
        let hook = RealDebugHook::new(vec![10]);
        assert!(!hook.should_pause(5, 1));
        assert!(!hook.did_pause());
    }

    #[test]
    fn should_pause_blocks_until_resume_is_called_from_another_thread() {
        let hook = Arc::new(RealDebugHook::new(vec![7]));
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause(7, 1));
        wait_paused(&hook);
        assert_eq!(hook.last_stop_reason(), Some(StopReason::Breakpoint));
        hook.resume();
        assert!(pause_thread.join().unwrap());
    }

    /// Stepping decides by depth: over skips deeper statements, into stops at them, out skips
    /// everything until the depth drops.
    #[test]
    fn steps_stop_by_call_depth() {
        let hook = Arc::new(RealDebugHook::new(vec![1]));
        let h = Arc::clone(&hook);
        // (line, depth) the program reaches, and which it should stop at.
        let program = std::thread::spawn(move || {
            [(1, 1), (5, 2), (2, 1), (5, 2), (6, 2), (3, 1)]
                .into_iter()
                .filter(|&(line, depth)| h.should_pause(line, depth))
                .map(|(line, _)| line)
                .collect::<Vec<_>>()
        });
        wait_paused(&hook); // line 1: breakpoint
        hook.step(StepKind::Over); // runs line 5 (deeper), stops at 2
        std::thread::sleep(Duration::from_millis(20));
        wait_paused(&hook);
        assert_eq!(hook.paused_at_line(), Some(2));
        assert_eq!(hook.last_stop_reason(), Some(StopReason::Step));
        hook.step(StepKind::Into); // stops at the call's first statement
        std::thread::sleep(Duration::from_millis(20));
        wait_paused(&hook);
        assert_eq!(hook.paused_at_line(), Some(5));
        hook.step(StepKind::Out); // runs 6 (same depth), stops at 3
        std::thread::sleep(Duration::from_millis(20));
        wait_paused(&hook);
        assert_eq!(hook.paused_at_line(), Some(3));
        hook.resume();
        assert_eq!(program.join().unwrap(), vec![1, 2, 5, 3]);
    }

    #[test]
    fn a_pause_request_stops_at_the_next_statement() {
        let hook = RealDebugHook::new(vec![]);
        assert!(!hook.should_pause(1, 1));
        hook.request_pause();
        let hook = Arc::new(hook);
        let h = Arc::clone(&hook);
        let t = std::thread::spawn(move || h.should_pause(2, 1));
        wait_paused(&hook);
        assert_eq!(hook.last_stop_reason(), Some(StopReason::Pause));
        hook.resume();
        assert!(t.join().unwrap());
    }

    #[test]
    fn set_variable_checks_the_kind_and_queues_the_write() {
        let hook = Arc::new(RealDebugHook::new(vec![1]));
        hook.record_bindings(vec![
            Binding {
                name: "x".into(),
                value: "1".into(),
                writable: Some(LiteralKind::Namba),
            },
            Binding {
                name: "PI".into(),
                value: "3.14".into(),
                writable: None,
            },
        ]);
        assert!(
            hook.set_variable("x", DebugLiteral::Namba(2.0)).is_err(),
            "only while paused"
        );
        let h = Arc::clone(&hook);
        let t = std::thread::spawn(move || h.should_pause(1, 1));
        wait_paused(&hook);
        assert!(hook
            .set_variable("x", DebugLiteral::Neno("a".into()))
            .is_err());
        assert!(hook.set_variable("PI", DebugLiteral::Namba(3.0)).is_err());
        assert!(hook.set_variable("y", DebugLiteral::Namba(3.0)).is_err());
        hook.set_variable("x", DebugLiteral::Namba(9.0)).unwrap();
        assert_eq!(
            hook.current_bindings()[0].value,
            DebugLiteral::Namba(9.0).display()
        );
        hook.resume();
        t.join().unwrap();
        assert_eq!(
            hook.take_writes(),
            vec![("x".to_string(), DebugLiteral::Namba(9.0))]
        );
        assert!(hook.take_writes().is_empty());
    }

    #[test]
    fn stopping_releases_a_paused_program_and_skips_later_pauses() {
        let hook = Arc::new(RealDebugHook::new(vec![1, 2]));
        let h = Arc::clone(&hook);
        let t = std::thread::spawn(move || (h.should_pause(1, 1), h.should_stop()));
        wait_paused(&hook);
        hook.set_stop(true);
        assert_eq!(t.join().unwrap(), (true, true));
        assert!(!hook.should_pause(2, 1));
        hook.reset();
        assert!(!hook.should_stop());
    }
}
