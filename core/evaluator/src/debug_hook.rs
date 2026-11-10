//! Real step-through debugging support: the `DebugHook` trait `pata-dap` drives a running
//! program through, plus `RealDebugHook`, the actual (not test-only) implementation the
//! native code's host calls into at every statement of a debug build (`Opcode::Line`).
//!
//! Lives here (not in `pata-dap`) because `core/evaluator` must be able to implement this trait
//! without depending on `pata/dap` — this project's crates never have a `core/` crate depend on
//! a `pata/` one (`pata-dap` depends on `asili-evaluator`, the normal direction, matching
//! `pata-cli`/`pata-runner`'s existing precedent). `pata-dap` re-exports `DebugHook` from here
//! rather than defining its own copy, so both sides of the contract stay in sync by construction.

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

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugPauseReason {
    Breakpoint,
    Step,
    Pause,
}

#[derive(Clone, Copy)]
enum StepMode {
    Into,
    Over(u64),
    Out(u64),
}

/// A running program's debug-control surface — implemented by `RealDebugHook` below (wired into
/// native code's host by `run_main_with_debug_hook`) and, in `pata-dap`'s own test suite, by a canned test
/// double exercising the DAP protocol layer independent of a real running program.
pub trait DebugHook: Send + Sync {
    /// Called by native code's host immediately before `should_pause`, with a fresh snapshot of
    /// every currently-in-scope binding (innermost-scope-wins on a shadowed name) — so a real
    /// pause always has up-to-date data ready for a `variables` request before it can possibly
    /// block. Takes an already-formatted snapshot rather than live registers, keeping
    /// this trait's shape independent of the host's frames.
    fn record_bindings(&self, bindings: Vec<(String, String)>);
    fn record_writable_names(&self, _names: Vec<String>) {}
    /// Called before executing the statement at `line`. Returning `true` means the caller
    /// (the host) should treat execution as having genuinely paused and later resumed —
    /// a real implementation blocks the calling thread internally (e.g. on a channel recv or a
    /// condvar wait) until `resume()` is called from another thread, rather than the caller
    /// polling in a loop.
    fn should_pause(&self, line: usize) -> bool;
    /// Like `should_pause`, with the current call depth for stepping support.
    fn should_pause_at(&self, line: usize, _call_depth: u64) -> bool {
        self.should_pause(line)
    }
    fn resume(&self);
    fn set_variable(&self, _name: &str, _value: &str) -> Option<String> {
        None
    }
    fn evaluate(&self, expression: &str) -> Option<String> {
        self.current_bindings()
            .into_iter()
            .find_map(|(name, value)| (name == expression).then_some(value))
            .or_else(|| {
                expression
                    .parse::<f64>()
                    .ok()
                    .map(|number| format!("Namba({number:?})"))
            })
            .or_else(|| match expression {
                "KWELI" => Some("Ukweli(true)".to_string()),
                "SIYO_KWELI" => Some("Ukweli(false)".to_string()),
                _ => None,
            })
    }
    fn continue_execution(&self) {
        self.resume();
    }
    fn step_in(&self) {}
    fn step_over(&self) {}
    fn step_out(&self) {}
    fn request_pause(&self) {}
    fn pause_reason(&self) -> Option<DebugPauseReason> {
        None
    }
    fn pause_generation(&self) -> u64 {
        0
    }
    fn paused_at_line(&self) -> Option<usize> {
        None
    }
    fn restart_requested(&self) -> bool {
        false
    }
    fn restart(&self) {
        self.continue_execution();
    }
    /// Snapshot of currently-in-scope bindings, for a `variables` DAP request — whatever was
    /// last passed to `record_bindings`.
    fn current_bindings(&self) -> Vec<(String, String)>;
}

/// The real, evaluator-side `DebugHook`: pauses at configured breakpoint lines, blocking the
/// executing thread on a `Mutex<bool>` + `Condvar` (the same primitive `pata-dap`'s own test
/// double used before a real implementation existed) until `resume()` is called from the DAP
/// server's own request-handling thread. `record_bindings` is called by the host right
/// before `should_pause`, so a `variables` request issued while genuinely paused reflects the
/// real, live environment at the paused line — not canned data.
pub struct RealDebugHook {
    breakpoints: Mutex<Vec<usize>>,
    paused: Arc<(Mutex<bool>, std::sync::Condvar)>,
    bindings: Mutex<Vec<(String, String)>>,
    writable_names: Mutex<HashSet<String>>,
    did_pause: AtomicBool,
    step_mode: Mutex<Option<StepMode>>,
    pause_requested: AtomicBool,
    paused_depth: AtomicI64,
    pause_reason: Mutex<Option<DebugPauseReason>>,
    pause_generation: AtomicI64,
    restart_requested: AtomicBool,
    /// The line currently paused at, or `-1` when not paused — a caller (e.g. `pata-dap`'s
    /// launch-monitor thread) polls this to know when/where to report a real pause, since
    /// `should_pause` itself blocks the calling (program) thread and can't be polled for its
    /// return value until it's already over.
    paused_at_line: AtomicI64,
}

impl RealDebugHook {
    pub fn new(breakpoints: Vec<usize>) -> Self {
        Self {
            breakpoints: Mutex::new(breakpoints),
            paused: Arc::new((Mutex::new(false), std::sync::Condvar::new())),
            bindings: Mutex::new(Vec::new()),
            writable_names: Mutex::new(HashSet::new()),
            did_pause: AtomicBool::new(false),
            step_mode: Mutex::new(None),
            pause_requested: AtomicBool::new(false),
            paused_depth: AtomicI64::new(-1),
            pause_reason: Mutex::new(None),
            pause_generation: AtomicI64::new(0),
            restart_requested: AtomicBool::new(false),
            paused_at_line: AtomicI64::new(-1),
        }
    }

    pub fn set_breakpoints(&self, lines: Vec<usize>) {
        *crate::sync::lock(&self.breakpoints) = lines;
    }

    /// Whether execution has genuinely paused at least once — real observability for a caller
    /// that wants to confirm a breakpoint actually fired, not just that `should_pause` was
    /// called (mirrors the same-purpose flag the earlier canned test double had).
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

    pub fn reset_for_restart(&self) {
        self.restart_requested.store(false, Ordering::SeqCst);
        *crate::sync::lock(&self.step_mode) = None;
        self.pause_requested.store(false, Ordering::SeqCst);
        self.paused_at_line.store(-1, Ordering::SeqCst);
        self.paused_depth.store(-1, Ordering::SeqCst);
        *crate::sync::lock(&self.pause_reason) = None;
        *crate::sync::lock(&self.bindings) = Vec::new();
        *crate::sync::lock(&self.writable_names) = HashSet::new();
        self.did_pause.store(false, Ordering::SeqCst);
    }
}

impl Default for RealDebugHook {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl DebugHook for RealDebugHook {
    fn record_bindings(&self, bindings: Vec<(String, String)>) {
        *crate::sync::lock(&self.bindings) = bindings;
    }

    fn record_writable_names(&self, names: Vec<String>) {
        *crate::sync::lock(&self.writable_names) = names.into_iter().collect();
    }

    fn should_pause(&self, line: usize) -> bool {
        self.should_pause_at(line, 0)
    }

    fn should_pause_at(&self, line: usize, call_depth: u64) -> bool {
        let breakpoint = crate::sync::lock(&self.breakpoints).contains(&line);
        let pause_requested = self.pause_requested.swap(false, Ordering::SeqCst);
        let step = {
            let mode = crate::sync::lock(&self.step_mode);
            match *mode {
                Some(StepMode::Into) => true,
                Some(StepMode::Over(depth)) => call_depth <= depth,
                Some(StepMode::Out(depth)) => call_depth < depth,
                None => false,
            }
        };
        if !breakpoint && !pause_requested && !step {
            return false;
        }

        let reason = if breakpoint {
            DebugPauseReason::Breakpoint
        } else if pause_requested {
            DebugPauseReason::Pause
        } else {
            DebugPauseReason::Step
        };
        *crate::sync::lock(&self.step_mode) = None;
        *crate::sync::lock(&self.pause_reason) = Some(reason);
        self.pause_generation.fetch_add(1, Ordering::SeqCst);
        self.paused_depth.store(call_depth as i64, Ordering::SeqCst);
        self.did_pause.store(true, Ordering::SeqCst);
        self.paused_at_line.store(line as i64, Ordering::SeqCst);
        let (lock, cvar) = &*self.paused;
        let mut is_paused = crate::sync::lock(&lock);
        *is_paused = true;
        while *is_paused {
            is_paused = cvar
                .wait(is_paused)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        self.paused_at_line.store(-1, Ordering::SeqCst);
        true
    }

    fn resume(&self) {
        let (lock, cvar) = &*self.paused;
        let mut is_paused = crate::sync::lock(&lock);
        *is_paused = false;
        cvar.notify_all();
    }

    fn set_variable(&self, name: &str, value: &str) -> Option<String> {
        if !crate::sync::lock(&self.writable_names).contains(name) {
            return None;
        }
        let mut bindings = crate::sync::lock(&self.bindings);
        let (_, current) = bindings.iter_mut().find(|(binding, _)| binding == name)?;
        let updated = if current.starts_with("Namba(") {
            let number = value.parse::<f64>().ok()?;
            format!("Namba({number:?})")
        } else if current.starts_with("Ukweli(") {
            match value {
                "KWELI" => "Ukweli(true)".to_string(),
                "SIYO_KWELI" => "Ukweli(false)".to_string(),
                _ => return None,
            }
        } else {
            return None;
        };
        *current = updated.clone();
        Some(updated)
    }

    fn evaluate(&self, expression: &str) -> Option<String> {
        let expression = expression.trim();
        if let Some((_, value)) = crate::sync::lock(&self.bindings)
            .iter()
            .find(|(name, _)| name == expression)
        {
            return Some(value.clone());
        }
        if let Ok(number) = expression.parse::<f64>() {
            return Some(format!("Namba({number:?})"));
        }
        match expression {
            "KWELI" => Some("Ukweli(true)".to_string()),
            "SIYO_KWELI" => Some("Ukweli(false)".to_string()),
            _ => None,
        }
    }

    fn continue_execution(&self) {
        *crate::sync::lock(&self.step_mode) = None;
        self.pause_requested.store(false, Ordering::SeqCst);
        self.resume();
    }

    fn step_in(&self) {
        *crate::sync::lock(&self.step_mode) = Some(StepMode::Into);
    }

    fn step_over(&self) {
        let depth = self.paused_depth.load(Ordering::SeqCst).max(0) as u64;
        *crate::sync::lock(&self.step_mode) = Some(StepMode::Over(depth));
    }

    fn step_out(&self) {
        let depth = self.paused_depth.load(Ordering::SeqCst).max(0) as u64;
        *crate::sync::lock(&self.step_mode) = Some(StepMode::Out(depth));
    }

    fn request_pause(&self) {
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    fn pause_reason(&self) -> Option<DebugPauseReason> {
        *crate::sync::lock(&self.pause_reason)
    }

    fn pause_generation(&self) -> u64 {
        self.pause_generation.load(Ordering::SeqCst) as u64
    }

    fn paused_at_line(&self) -> Option<usize> {
        RealDebugHook::paused_at_line(self)
    }

    fn restart_requested(&self) -> bool {
        self.restart_requested.load(Ordering::SeqCst)
    }

    fn restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst);
        self.resume();
    }

    fn current_bindings(&self) -> Vec<(String, String)> {
        crate::sync::lock(&self.bindings).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pause_returns_false_for_a_line_with_no_breakpoint() {
        let hook = RealDebugHook::new(vec![10]);
        assert!(!hook.should_pause(5));
        assert!(!hook.did_pause());
    }

    #[test]
    fn should_pause_blocks_until_resume_is_called_from_another_thread() {
        let hook = Arc::new(RealDebugHook::new(vec![7]));
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause(7));

        for _ in 0..100 {
            if hook.did_pause() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            hook.did_pause(),
            "should_pause should have started blocking by now"
        );

        hook.resume();
        assert!(pause_thread.join().unwrap());
    }

    #[test]
    fn step_over_skips_deeper_calls_and_stops_at_the_next_same_depth_line() {
        let hook = Arc::new(RealDebugHook::new(vec![1]));
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause_at(1, 1));
        while !hook.did_pause() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        hook.step_over();
        hook.resume();
        assert!(!hook.should_pause_at(2, 2));
        let hook_for_step = Arc::clone(&hook);
        let step_thread = std::thread::spawn(move || hook_for_step.should_pause_at(3, 1));
        while hook.paused_at_line() != Some(3) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(hook.pause_reason(), Some(DebugPauseReason::Step));
        hook.resume();
        assert!(pause_thread.join().unwrap());
        assert!(step_thread.join().unwrap());
    }

    #[test]
    fn step_in_stops_at_the_next_line_at_any_call_depth() {
        let hook = Arc::new(RealDebugHook::new(vec![1]));
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause_at(1, 1));
        while !hook.did_pause() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        hook.step_in();
        hook.resume();
        let hook_for_step = Arc::clone(&hook);
        let step_thread = std::thread::spawn(move || hook_for_step.should_pause_at(2, 2));
        while hook.paused_at_line() != Some(2) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(hook.pause_reason(), Some(DebugPauseReason::Step));
        hook.resume();
        assert!(pause_thread.join().unwrap());
        assert!(step_thread.join().unwrap());
    }

    #[test]
    fn step_out_waits_for_a_shallower_call_depth() {
        let hook = Arc::new(RealDebugHook::new(vec![1]));
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause_at(1, 2));
        while !hook.did_pause() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        hook.step_out();
        hook.resume();
        assert!(!hook.should_pause_at(2, 2));
        let hook_for_step = Arc::clone(&hook);
        let step_thread = std::thread::spawn(move || hook_for_step.should_pause_at(3, 1));
        while hook.paused_at_line() != Some(3) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        hook.resume();
        assert!(pause_thread.join().unwrap());
        assert!(step_thread.join().unwrap());
    }

    #[test]
    fn pause_request_stops_at_the_next_line() {
        let hook = Arc::new(RealDebugHook::new(Vec::new()));
        hook.request_pause();
        let hook_for_pause = Arc::clone(&hook);
        let pause_thread = std::thread::spawn(move || hook_for_pause.should_pause_at(8, 1));
        while hook.paused_at_line() != Some(8) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(hook.pause_reason(), Some(DebugPauseReason::Pause));
        hook.resume();
        assert!(pause_thread.join().unwrap());
    }
}
