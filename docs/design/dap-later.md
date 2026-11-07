# DAP (Debug Adapter Protocol)

**Real step-through debugging works end-to-end.**

`pata/dap` is a real, tested binary crate (`pata-dap`) implementing: `initialize`, `launch`,
`setBreakpoints`, `configurationDone`, `continue`, `next`, `stepIn`, `stepOut`, `pause`,
`stackTrace`, `scopes`, `variables`, `setVariable`, `evaluate`, `restart`, `threads`,
`disconnect`. `attach` is answered with an error (see below). It runs over real stdio using the
`dap` crate for wire-protocol (de)serialization, structurally mirroring `pata-lsp/src/server.rs`'s
shape even though DAP and LSP are different protocols.

## What's real

- The full request/response/event protocol handling, verified end-to-end against real
  `Content-Length`-framed wire bytes (`pata/dap/src/server.rs`'s own tests).
- **Events from the program's side.** Each pause sends a `stopped` event (reason `breakpoint`,
  `step` or `pause`); the run's end sends `exited` (code 0, or 1 if the program failed) and
  `terminated`; a `launch` whose file can't be read or parsed sends an `output` event (stderr)
  saying why, then `terminated`. `run_session` writes these from a second thread while the
  request loop waits for input (`DapSession::events` is the sink the program's side holds).
- **A real `DebugHook` implementation, in `core/evaluator` itself**
  (`asili_evaluator::debug_hook::RealDebugHook`). A debug build
  (`CompileOptions { lines: true, bindings: true }`) emits `Opcode::Line` before every statement,
  naming the locals visible there; native code's host snapshots them into the hook and calls
  `should_pause(line, depth)`. A pause blocks the program's thread on a `Mutex` + `Condvar` (not
  polling) until the debugger resumes it from the request thread.
- **Stepping is by call depth.** A debug build inlines no call, so the host's call depth is exact
  at every statement: `stepIn` stops at the very next statement; `next` at the next one no deeper
  than where it started (calls run through); `stepOut` at the next one shallower. `pause` stops at
  the next statement the program reaches.
- **`setVariable`** writes a local that holds a `Namba`, `Ukweli` or `Neno`. The value is an Asili
  literal (a number, optionally negative, `"text"` with the language's escapes, `kweli`,
  `si_kweli`), read by the language's lexer, and must be the kind the local already holds; other
  values and predefined names are read-only. The write is queued while paused and the host stores
  it into the local's register when the program resumes. In a debug build `native::num_writes`
  lists `Line`'s numeric binds, so native code reloads those registers after `Line` and range
  analysis assumes nothing about them. Release and coverage builds are unaffected: they emit no
  binds.
- **`evaluate`** (hover, watch, REPL) returns the value of a name visible at the paused
  statement, or a literal's own value. Other expressions get an error: evaluating them would mean
  compiling code into the paused native program.
- **`restart`** ends the current run (the hook's stop flag; the host fails the run at its next
  statement, releasing it if paused), waits for its thread, and launches again without reporting
  the old run's end. **`disconnect`** ends the run the same way.
- **`pata-dap` actually launches and runs the target program.** A `launch` naming a `.as` file,
  followed by `setBreakpoints` and `configurationDone`, compiles that file (lex → parse — single
  file only, no project/dependency resolution) and runs it with `RealDebugHook` attached via
  `asili_evaluator::run_main_with_debug_hook` (`pata/dap/src/runner.rs`). `setBreakpoints` during
  a run takes effect from the program's next statement.
- Verified by tests: `core/evaluator/tests/debug_hook.rs` steps through a real program across
  calls, writes a numeric and a text local and checks what the program then computes, and stops
  a paused run. `pata/dap/src/runner.rs` drives a whole session against a real file: breakpoint →
  `stopped`, `next` → `stopped` on the next line, `restart` → `stopped` at the breakpoint again
  with no `terminated` for the old run, `continue` → `exited` 0 + `terminated`.

## Why `attach` is refused

An Asili program can be debugged only when it is built as a debug build, which marks every
statement. Programs built normally (`pata jenga`, `pata tenda`) have no such marks and no
debugger connection, so there is no running process an `attach` could join. Debug by `launch`.

## Where `DebugHook` lives, and why

`core/` crates never depend on `pata/` crates in this project — but `core/evaluator` is the crate
whose host calls the hook, so the trait lives in `core/evaluator/src/debug_hook.rs`. `pata-dap`
depends on `asili-evaluator` (the normal direction) and re-exports `DebugHook` from there rather
than keeping a second copy.

```rust
pub trait DebugHook: Send + Sync {
    fn record_bindings(&self, bindings: Vec<Binding>);
    fn should_pause(&self, line: usize, depth: u64) -> bool;
    fn resume(&self);
    fn current_bindings(&self) -> Vec<Binding>;
    fn step(&self, kind: StepKind);
    fn request_pause(&self);
    fn set_variable(&self, name: &str, value: DebugLiteral) -> Result<(), String>;
    fn take_writes(&self) -> Vec<(String, DebugLiteral)>;
    fn should_stop(&self) -> bool;
    fn set_breakpoints(&self, lines: Vec<usize>);
}
```

A `Binding` carries the name, the shown value, and the kind a write must have (`None`:
read-only). `write_binding` is the one rule for what may be written, shared by `RealDebugHook`
and the test double.

`MockHook` (`pata/dap/src/mock_hook.rs`) still exists: a canned test double for exercising the DAP
wire protocol independent of compiling/running a real program.

## What's still out of scope

- Conditional breakpoints, function breakpoints, exception breakpoints. Breakpoints are
  line-only, statement-granularity (#139).
- `evaluate` of anything beyond a visible name or a literal.
- `stackTrace` reports one frame (named `kuu`) at the paused line. It does not list the call
  chain or the paused function's real name (#138).
- The program's own output goes to `pata-dap`'s stdout, which is also the DAP channel. A program
  that prints will corrupt the protocol stream. Routing it into `output` events is open work (#137).
- `launch` compiles a single file only (no `leta`-import resolution via `pata-core`, no
  `pata.toml`-aware project build).

See `docs/design/pata-implementation-spec.md` Section 20 for the original design rationale and
`docs/design/implementation-status.md` for where this sits relative to other work.
