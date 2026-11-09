# Peripheral I/O: exploration

**Status: partly implemented.** The virtual board (`ubao`, "Built on the playground" below) and
its device path on the ESP32 ("Device path on the ESP32" below) exist; serial output, timers, the
trace comparison across targets and Cortex-M do not. It records what the code already allows, the options for letting Asili programs drive pins, serial lines and timers, and
the spikes that would decide between them. Related: [wazi-hardware-design.md](wazi-hardware-design.md)
(the unsafe-block and hardware-handle design), [safety-critical-roadmap.md](safety-critical-roadmap.md),
and [docs/spec/05-standard-library.md](../spec/05-standard-library.md) (the `piga_pini` and `Pini`
names this would implement).

## Goal

A program written in Asili should drive a pin, write a serial line and read a clock, and the same
program with the same inputs should produce the same sequence of I/O events on every target:

1. the browser playground, against a virtual board drawn on the page;
2. a Cortex-M device, or QEMU standing in for one;
3. an external simulator, if one is worth hosting (Wokwi custom chips is the candidate).

"Same sequence of I/O events" is the new claim. Native code already has to agree with the
reference results bit for bit (`engines_agree.rs`, `native_tiers.rs`); this extends the same test
idea from return values to I/O traces.

## What the code already allows (verified)

- **Native code reaches the host through a fixed runtime table.** `native.rs` (struct around line 64)
  holds function pointers: `exec`, the list helpers, `fmod`, `pow`, `call_host`, and so on.
  `core/evaluator/src/nguvu/wasm.rs` mirrors it as `RT_COUNT = 19` entries with signatures in
  `runtime_signature`, and `nguvu/ir.rs` names them as `RtFn`. A new runtime entry is therefore a
  mechanical addition on all three sides, and changes the native ABI (`ABI_VERSION`, currently 11 in
  `core/evaluator/src/aot.rs`).
- **Everything else goes through `Exec`.** `RtFn::Exec` hands an instruction to the host
  (`host.rs`). The device runtime answers it with status `2` (failure), see
  `driver/kifaa/src/lib.rs` (`asili_kifaa_exec`). So a builtin that is not lowered into native code
  cannot run on a device at all. Any peripheral call has to be lowered, not left to `Exec`.
- **The wasm target imports only `env.memory`, `env.table` and `env.base`** (`nguvu/wasm.rs`,
  `module()`). Adding an `env` function import is a small change to the module's import section, and
  the browser worker (`asili-playground/worker.js`) supplies it. This is the cheapest way for the
  playground to receive pin writes.
- **Device code is `no_std`, no allocator, no threads, no files** (`driver/kifaa/src/lib.rs`
  header). A peripheral implementation there has no OS to lean on, which suits register access.
- **Builtins live in one registry** (`core/evaluator/src/builtins/mod.rs`, `BUILTIN_MODULES` in
  `core/parser/src/builtins.rs`). `syscall` is registered there with no `wazi` gate, which
  [wazi-hardware-design.md](wazi-hardware-design.md) already flags.

## What does not exist

- No `wazi` keyword, `Pini` type, `Bafa<T>` type or `piga_pini` function (grep confirms; the spec
  names them).
- No peripheral values in `Value` (`core/evaluator/src/value/mod.rs`) or `ValueType`
  (`core/parser/src/ast.rs`).
- No host-side event sink. Output today is `platform.rs` writing to stdout, or to `console.log` in
  the browser. There is nothing that records an ordered trace of I/O events.

## Where the interface should sit

Three layers, each with one owner:

| Layer | Contents | Owner |
|---|---|---|
| Language | `wazi` gate (or a capability check, see questions), `Pini` handle, `piga_pini(pin, level)`, read, serial write, `millis()` | `core/parser`, `core/evaluator` |
| Peripheral ABI | a small fixed set of operations: `gpio_set`, `gpio_get`, `uart_write`, `now_ms`, `delay_ms` | one Rust trait-like table in `core/evaluator` |
| Target implementation | virtual board (wasm import), MMIO (`driver/kifaa`), external simulator adapter | one file per target |

The language layer never names a target. The ABI is the only thing `nguvu` lowers to. Each target
supplies its implementation of the same operations, and the trace test runs the same program across
them.

## Options

**A. Runtime-table entries (device-first).** Add `gpio_set`, `gpio_get`, `uart_write`, `now_ms` as
new `RtFn` entries. The device implements them as MMIO; the host implements them as trace calls;
wasm uses an `env` import. Pro: native code calls them directly with no `Exec` round trip, so
timing is predictable (matters for `#[salama]`). Con: an ABI change and an entry in three tables
per operation.

**B. Builtin modules via `Exec` (host-first).** Register `kifaa`-style builtins and let the host run
them. Pro: quick, no ABI change. Con: fails on a device by construction (see above), so it cannot
meet the Cortex-M goal. Acceptable only for the playground spike.

**C. Wasm imports only (browser-first).** Only the wasm target gets peripherals, as `env` imports.
Pro: smallest change. Con: the device and the trace test would have no peripheral path, so this
does not meet goals 2 and 3.

**Recommendation:** A for the operations the device needs, with B used only to prototype the
virtual board before A is wired. Choose after spike 1 (below) measures the wasm import cost.

## Decisions needed

1. **Is the `wazi` block a lexical scope or a capability?** Proposed resolution below ("wazi:
   grant and check"). Decided together with the `syscall` gate, as this section requires.
2. **Event shape and clock.** Proposed resolution below ("Time in traces").
3. **Clock.** `now_ms` must be virtual in the playground and tests (so traces reproduce) and real
   on a device. Make it a target operation, never a host clock read inside shared code.
4. **Scope of the first board.** A pin set, a serial line and a clock is enough for LED blink, a
   button and a serial log. Sensors and buses (I2C, SPI) wait.

## wazi: grant and check (proposed)

**Question:** a lexical scope, or a capability at each call site?

**Findings.** The repo already has both halves:

- `#[salama]` is a *function* attribute, and `core/evaluator/src/salama.rs` checks it over the
  call graph: a strict function may call only strict functions, and recursion is rejected even
  when it goes through other functions. A lexical marker alone would not hold, because a helper
  outside the marked region could do the forbidden thing.
- The `syscall` builtin's own description in `core/parser/src/builtins.rs` says "ndani ya `wazi`"
  (inside `wazi`), but nothing in the analyzer or evaluator checks it. The docs already promise a
  gate the code does not enforce. The same is true of `kiungo`, which is documented as FFI and is
  not gated at all.

**Proposal.** A grant is lexical; the check is transitive, following the `#[salama]` model.

1. **Operations that need a grant:** `syscall`, `kiungo` calls, `tenga_kumbukumbu`,
   `anwani_ya`, `piga_pini`, `vuta_vifaani`, and dereferencing `Kiashiria`.
2. **Grants:** a `wazi { ... }` block, or a function marked `#[wazi]`. Use an attribute for the
   function form, not a new keyword, so the attribute check in `analyzer.rs` (`check_attrs`) covers
   it. Only the block needs the keyword `wazi`, which the toolchain sync rules cover.
3. **Check at the call site:** a tagged operation outside a grant is a compile error. Calling a
   `#[wazi]` function from outside a grant is also an error, the way Rust treats `unsafe fn`.
4. **Exclusion:** `#[salama]` and `#[wazi]` cannot be combined. A syscall has no time bound, so the
   two guarantees contradict each other.
5. **Not a token:** an object-capability (the program is handed a `Pini` and only it can drive that
   pin) is attractive for pins, because the host grants exactly what a program may touch. It does
   not fit `syscall`, which has no natural token. Keep it as a later refinement for `Pini` only.

**Why not pure call-site capability now:** it needs a token threaded through every signature, which
is a larger change than the analyzer work above and gives no benefit for the first board.

**Immediate, independent fix:** the `syscall` builtin is documented as gated but is not. Gating it
under this rule is a bug fix, not a feature, and can land before any peripheral work.

## Time in traces (proposed)

**Question:** is `tick` a step count or wall time?

**Findings.**

- Wall time already leaks into programs: `core/evaluator/src/builtins/majira.rs` reads
  `SystemTime`. Any program that uses it is not reproducible, with or without peripherals.
- Statement counting does not exist. `#[salama]` *bounds* steps statically; nothing counts them at
  run time. Counting them would need an instruction in every lowered statement, changing native
  code, the wasm target and the device runtime, and the Sudoku numbers that
  `performance-guardrails` protects. It would also drift when the optimiser changes, because
  counts would follow the IR rather than the source.

**Proposal: three clocks, each with one job.**

| Clock | Definition | Used for | Reproducible? |
|---|---|---|---|
| `hatua` (event index) | number of I/O events so far, from 0 | ordering key and the trace equality test | yes |
| virtual time | sum of the delays the program requests (`delay_ms`) | timestamps in traces and the playground display | yes |
| wall time | the real clock | live display and profiling only; never compared | no |

- The trace equality test compares `(hatua, kind, pin, value, virtual time)`. It needs no
  instrumentation: the event index is a count the I/O layer keeps.
- Virtual time advances only by requested delays. Compute time is not modelled, so two targets can
  show the same trace while differing in real elapsed time between events. That limitation should
  be stated in user-facing docs, not hidden.
- In trace mode, `majira` returns virtual time, so programs that read the clock stay reproducible.
  Outside trace mode it keeps the real clock.
- A statement-level counter stays an option, but only if a test shows that event indexes cannot
  separate two behaviours a learner needs to see.

## Spikes (each decides a question above)

1. **Wasm import cost.** Extend `nguvu/wasm.rs` with one `env.gpio_set(i32, i32)` import and call
   it from a loop of a million iterations in the playground. Record the time against the current
   build. Decides option A vs B for the browser path.
2. **MMIO on QEMU.** Implement `gpio_set` as a volatile write to a fixed address in `driver/kifaa`,
   and run a blink program under `qemu-arm -cpu cortex-m7` with a load address that the QEMU
   machine maps. Decides whether the device path works without new hardware.
3. **Trace equality.** Make both spikes record `(tick, kind, pin, value)` for the same program and
   compare. Success: identical traces across the wasm and QEMU runs. This is the first real test of
   the differentiator.
4. **Wokwi chip ABI (reading, not code).** Read the official Chips API reference
   (docs.wokwi.com/chips-api) and confirm the import names, the `chip_init` entry, and the license
   and terms for hosting or embedding a chip. Do not implement against the beta API until this is
   read.

## Risks

- **Scope creep into a HAL.** Keep to the four operations above until a real program needs more.
- **Timing claims.** `#[salama]` bounds steps, not microseconds. A peripheral call's cost must be
  measured on the device before any real-time claim.
- **Two definitions of the same operation.** The virtual board and the MMIO version must share the
  event sink code in one place (see the "write once" table in `CLAUDE.md`), not two copies.
- **External simulator terms and APIs.** Wokwi's API is beta and Velxio is AGPLv3. Neither is to be
  copied into Asili until a licence decision is made.

## Not in scope here

Market and user research, board libraries, interrupts, DMA, and anything touching `pata` beyond
what the spikes need.

## Device path on the ESP32 (implemented)

Option A in a form that needs no change to the host's runtime ABI:

1. **Typed instructions.** `ubao_andika`, `ubao_soma`, `ubao_subiri` and `ubao_saa` called on
   numbers (and a truth value) compile to bytecode instructions of their own — `PinWrite`,
   `PinRead`, `BoardWait`, `BoardClock` (`BYTECODE_VERSION` 23) — the way `sakafu`/`dari` became
   `Floor`/`Ceil`. The host runs them with the builtins' own code
   (`builtins::ubao::{andika, soma, subiri, saa}`), so the virtual board has one implementation.
2. **Device lowering.** In device code (`lower::Ctx::device`) they lower to runtime calls with
   their operands (`RtFn::PinWrite`, `PinRead`, `BoardWait`, `BoardClock`); host code still hands them to the
   host, so no runtime-table entry was added.
3. **Target implementation.** The ESP32's runtime inlining (`device::inline_runtime`) turns the
   pin calls into memory-mapped register accesses (`Inst::IoLoad`/`IoStore`, `memw`-ordered) and
   the wait into a helper on the cycle counter (`Inst::CycleCount`), and the clock into a helper
   reading timer group 1's timer 0, which keeps the state (stopped: not started); Cortex-M
   rejects them.
4. **Entry point.** A `#[wazi] kuu` builds as `app_main`, so a firmware has no C of its own.

Pin numbers must be build-time constants on the device (checked: existing, not the flash's,
writable). The device clock is real time where the host's is virtual. The cross-target trace
test remains.

## Built on the playground (decided)

The virtual board is built on top of the procedural and dynamic playground, not beside it. The
playground's sample gallery is generated from its sample files (`scripts/gen_samples.py` in
`asili-playground`), so board scenarios follow the same pattern: each scenario is a file, the
picker is generated, and CI checks that the generated list is current.

Consequences for the design:

1. **Virtual board = a builtin module, run through the host.** Native code in the browser reaches
   builtins through `Exec`, which works there (it is the device that lacks a host). So the first
   board needs no runtime-table change and no wasm import. This is spike 1's fallback path, and it
   is acceptable because the board is browser-only. The device path (option A) comes later.
2. **Pin operations are `#[wazi]`.** The approved grant rule applies: a `kazi` that drives a pin
   must be marked `#[wazi]`, the same as `syscall`.
3. **Inputs are scripted, not live.** A scenario lists pin levels at virtual times. A read returns
   the level the scenario gives at the current virtual time, so a run is reproducible and the trace
   test can use the same scenario on every target.
4. **Trace transport.** Events leave the worker tagged on their own channel, separate from program
   text, so the page can draw the board without parsing output. The wasm-browser shim is the one
   place this is defined.

Build order (each step lands separately, with tests):

- **Step 1 (Asili):** `ubao` builtin module: `andika(pini, hali)`, `soma(pini)`, `subiri(ms)`,
  `saa()` (virtual time). Registered in `BUILTIN_MODULES` with Swahili descriptions; stubs
  regenerated; `#[wazi]` required for `andika` and `soma`; `engines_agree` and `native_tiers` tests
  for the new calls; `performance-guardrails` checks; changelog.
- **Step 2 (Asili):** the event sink in `platform.rs` (wasm-browser: tagged `console` lines; native:
  stdout off by default) and a trace test that replays a scenario and compares the event list
  across the native and wasm builds.
- **Step 3 (playground):** scenario files (`scenarios/*.as` with a `# scenario:` header and a
  scripted input list), generated picker, and a board panel that draws pins over virtual time.
- **Step 4 (playground):** trace view and the scenario gallery's CI check.

Open choices, with the defaults I would take unless told otherwise:

- Module name `ubao` (board), in line with the existing Swahili module names.
- Scripted inputs in a `# input:` header or a separate `.json` next to each scenario: a header, so
  one file holds one scenario.
- Event tag `\u0001ASILI-EVT ` followed by one JSON object per line.
