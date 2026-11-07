//! End-to-end proof that `Runtime::debug_hook` actually reaches a running program — not just
//! that `RealDebugHook`'s own pause/resume/bindings mechanism works in isolation (covered by
//! `core/evaluator/src/debug_hook.rs`'s own unit tests), but that `eval_stmt_impl` genuinely
//! calls it once per statement, with real (not canned) bindings, on a real interpreter thread
//! that can be resumed from another thread — the actual wiring issue #43 closes.

use std::sync::Arc;
use std::time::Duration;

use asili_evaluator::debug_hook::{DebugHook, DebugLiteral, RealDebugHook, StepKind, StopReason};
use asili_evaluator::run_main_with_debug_hook;
use asili_lexer::tokenize;
use asili_parser::parse_tokens;

fn compile(src: &str) -> asili_parser::Module {
    let tokens = tokenize(src).expect("tokenize");
    parse_tokens(&tokens).expect("parse")
}

/// A breakpoint on a real line inside `kuu` genuinely pauses the executing thread until
/// `resume()` is called from another thread — proven the same way `MockHook`'s own test proved
/// it, but through the real evaluator, not a hand-rolled fake standing in for one.
#[test]
fn breakpoint_pauses_a_real_running_program_until_resumed() {
    let module = compile(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka a = 1\n\
           weka b = 2\n\
           weka c = a + b\n\
         }",
    );
    // Line 3 ("weka b = 2") is the breakpoint.
    let hook = Arc::new(RealDebugHook::new(vec![3]));
    let hook_for_run = Arc::clone(&hook);

    // EvalError/Value aren't Send (the evaluator's Value can hold an Rc<RefCell<..>>, e.g. for
    // Mkondo handles), matching the same constraint run_test_with_timeout already works around —
    // convert to a Send-safe bool before crossing the thread boundary rather than propagating
    // the raw Result.
    let (tx, rx) = std::sync::mpsc::channel();
    let run_thread = std::thread::spawn(move || {
        let ok = run_main_with_debug_hook(&module, vec![], hook_for_run).is_ok();
        let _ = tx.send(ok);
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !hook.did_pause() {
        assert!(
            std::time::Instant::now() < deadline,
            "breakpoint never fired within 5s"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        hook.did_pause(),
        "should_pause must have actually blocked at line 3"
    );

    hook.resume();
    let ok = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("run thread should finish after resume");
    assert!(ok, "program should finish successfully after resuming");
    run_thread.join().expect("run thread should not panic");
}

/// The actual `variables` use case: while genuinely paused at a breakpoint, `current_bindings`
/// must reflect the real, live local variables at that point in execution — not canned/stale
/// data, and specifically *not* a variable defined only later in the function (proving the
/// snapshot really is taken at the paused line, not at the end of the run).
#[test]
fn current_bindings_reflect_real_state_while_paused() {
    let module = compile(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka x = 42\n\
           weka y = 7\n\
         }",
    );
    // Line 3 ("weka y = 7") — by the time this line is *about* to execute, x is already bound
    // but y is not yet.
    let hook = Arc::new(RealDebugHook::new(vec![3]));
    let hook_for_run = Arc::clone(&hook);

    let (tx, rx) = std::sync::mpsc::channel();
    let run_thread = std::thread::spawn(move || {
        let ok = run_main_with_debug_hook(&module, vec![], hook_for_run).is_ok();
        let _ = tx.send(ok);
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !hook.did_pause() {
        assert!(
            std::time::Instant::now() < deadline,
            "breakpoint never fired within 5s"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let bindings = hook.current_bindings();
    assert!(
        bindings
            .iter()
            .any(|b| b.name == "x" && b.value.contains("42")),
        "expected x=42 to already be bound while paused at line 3, got: {bindings:?}"
    );
    assert!(
        !bindings.iter().any(|b| b.name == "y"),
        "y is defined by the statement about to execute (not yet run) — it must not appear yet, got: {bindings:?}"
    );

    hook.resume();
    let _ = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("run thread should finish after resume");
    run_thread.join().expect("run thread should not panic");
}

/// A program with no breakpoints configured must run to completion exactly as if no hook were
/// attached at all — `should_pause` returning `false` on every line must never block.
#[test]
fn no_configured_breakpoints_runs_to_completion_without_blocking() {
    let module = compile(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka a = 1\n\
           weka b = 2\n\
         }",
    );
    let hook = Arc::new(RealDebugHook::new(vec![]));

    let result = run_main_with_debug_hook(&module, vec![], hook);
    assert!(
        result.is_ok(),
        "a run with no breakpoints must complete normally: {result:?}"
    );
}

/// Shadowing: while paused, an inner binding hides an outer one of the same name — the
/// variables view shows each name once, with the value the paused line would read.
#[test]
fn inner_bindings_shadow_outer_ones_while_paused() {
    let module = compile(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka x = 1\n\
           ikiwa kweli {\n\
             weka x = 2\n\
             weka y = x\n\
           }\n\
         }",
    );
    let hook = Arc::new(RealDebugHook::new(vec![5]));
    let hook_for_run = Arc::clone(&hook);
    let run_thread =
        std::thread::spawn(move || run_main_with_debug_hook(&module, vec![], hook_for_run).is_ok());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !hook.did_pause() {
        assert!(
            std::time::Instant::now() < deadline,
            "breakpoint never fired"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let bindings = hook.current_bindings();
    let xs: Vec<_> = bindings.iter().filter(|b| b.name == "x").collect();
    assert_eq!(xs.len(), 1, "{bindings:?}");
    assert!(xs[0].value.contains('2'), "{bindings:?}");
    hook.resume();
    assert!(run_thread.join().unwrap());
}

fn wait_paused_at(hook: &RealDebugHook, line: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while hook.paused_at_line() != Some(line) {
        assert!(
            std::time::Instant::now() < deadline,
            "never paused at line {line} (now {:?})",
            hook.paused_at_line()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Step over runs a call through; step into enters it; step out returns to the caller — on a
/// real running program, where the host's call depth decides.
#[test]
fn stepping_through_a_real_program_follows_calls_by_depth() {
    let module = compile(
        "kazi ongeza(n: Namba) -> Namba {\n\
           weka m = n + 1\n\
           rejesha m\n\
         }\n\
         kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka a = ongeza(1)\n\
           weka b = ongeza(a)\n\
           weka c = a + b\n\
         }",
    );
    let hook = Arc::new(RealDebugHook::new(vec![6]));
    let h = Arc::clone(&hook);
    let run = std::thread::spawn(move || run_main_with_debug_hook(&module, vec![], h).is_ok());

    wait_paused_at(&hook, 6);
    hook.step(StepKind::Over);
    wait_paused_at(&hook, 7);
    assert_eq!(hook.last_stop_reason(), Some(StopReason::Step));
    hook.step(StepKind::Into);
    wait_paused_at(&hook, 2);
    hook.step(StepKind::Out);
    wait_paused_at(&hook, 8);
    hook.resume();
    assert!(run.join().unwrap());
}

/// A value written while paused is the value the program goes on with: numeric (a register
/// native code holds) and text alike.
#[test]
fn set_variable_changes_what_the_program_computes() {
    let path = std::env::temp_dir().join(format!("asili-dbg-set-{}", std::process::id()));
    let module = compile(&format!(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {{\n\
           weka a = 1\n\
           weka s = \"x\"\n\
           weka b = a + 1\n\
           weka r = andika_faili(\"{}\", s + (b kama Neno))\n\
         }}",
        path.display()
    ));
    let hook = Arc::new(RealDebugHook::new(vec![4]));
    let h = Arc::clone(&hook);
    let run = std::thread::spawn(move || run_main_with_debug_hook(&module, vec![], h).is_ok());

    wait_paused_at(&hook, 4);
    hook.set_variable("a", DebugLiteral::Namba(41.0)).unwrap();
    hook.set_variable("s", DebugLiteral::Neno("y".into()))
        .unwrap();
    assert!(hook.set_variable("a", DebugLiteral::Ukweli(true)).is_err());
    hook.resume();
    assert!(run.join().unwrap());
    let written = std::fs::read_to_string(&path).expect("program wrote its result");
    let _ = std::fs::remove_file(&path);
    assert_eq!(written, "y42");
}

/// Stopping (restart/disconnect) ends a paused run with an error instead of finishing it.
#[test]
fn stop_ends_a_paused_run() {
    let module = compile(
        "kazi kuu(hoja: Orodha<Neno>) -> Tupu {\n\
           weka a = 1\n\
           weka b = 2\n\
         }",
    );
    let hook = Arc::new(RealDebugHook::new(vec![2]));
    let h = Arc::clone(&hook);
    let run = std::thread::spawn(move || run_main_with_debug_hook(&module, vec![], h).is_err());
    wait_paused_at(&hook, 2);
    hook.set_stop(true);
    assert!(
        run.join().unwrap(),
        "a stopped run must not finish normally"
    );
}
