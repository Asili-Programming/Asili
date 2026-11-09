# `wazi` (unsafe block) and hardware primitives design

**Status: partly implemented.** The analyzer enforces `#[wazi]` as a function-level grant:
`syscall` and the board's pin calls (`ubao_andika`, `ubao_soma`) require it (SEM121), and a
`#[wazi]` function may not also be `#[salama]` (SEM122). The virtual board is implemented, and
the ESP32 device target drives GPIO and reads/waits on its hardware clock; `#[wazi] kuu` is the
ESP-IDF firmware entry point. The `wazi { ... }` block keyword is still not in the lexer or
parser. The spec's `Kiashiria`, `Pini`, `Bafa<T>` and additional hardware functions
(`vuta_vifaani`, `anwani_ya`, `tenga_kumbukumbu`, `piga_pini`) are not implemented as such. The
grant design is in
[peripheral-io-design.md](peripheral-io-design.md). Phase IV, per
[docs/spec/07-execution-and-roadmap.md](../spec/07-execution-and-roadmap.md). This doc is
forward-looking, with the remaining design questions below.

## What `wazi` is supposed to gate

Per the spec, `wazi` is Asili's unsafe-block equivalent — the explicit escape hatch for manual
memory management and hardware access that the language's default ownership model otherwise
forbids. Three spec sections describe what lives inside it:

- **`01-philosophy-and-edp.md`**: "`wazi` (unsafe) blocks remain the escape hatch for manual
  memory and pointers" — positioned as the *only* sanctioned way to step outside
  ownership/borrowing, paralleling Rust's `unsafe`.
- **`04-type-system.md`**: `Biti8`/`Biti16`/`Biti32`/`Biti64`/`uBiti*` fixed-width integer types
  ("used inside `wazi` blocks or when performance/memory is critical" — so these are usable
  outside `wazi` too, unlike the rest of this list) and `Kiashiria` (raw pointer, "used only in
  `wazi` blocks").
- **`05-standard-library.md`**: hardware-control functions (`vuta_vifaani` — accelerator
  offload, `anwani_ya` — address-of, `tenga_kumbukumbu` — manual heap allocation, `piga_pini` —
  GPIO) and two typed handles, `Pini` (GPIO pin) and `Bafa<T>` (aligned accelerator buffer).

`Kiashiria`, `Pini`, `Bafa<T>` and the four spec-named functions still have no implementation in
the type system or evaluator. GPIO functionality does exist under `ubao`, with a virtual-board
implementation and an ESP32 implementation. Unlike `Mfululizo`/`Seti`/`Namba_Kuu`/
`Namba_Sahihi` (see [data-shapes-design.md](data-shapes-design.md)), no type-level recognition
exists yet for the remaining handles.

## `syscall` gate

The earlier concern that `syscall` lacked a `wazi` gate is resolved: the semantic analyzer
rejects calls outside a `#[wazi]` function (SEM121). It remains a stub
(`FIXME(Phase IV)`: returns `Anuani(0)` without issuing a syscall). The same function-level grant
is used for `ubao` pin access; a lexical `wazi { ... }` block is not implemented.

## Design questions

- **What does `wazi { ... }` actually change, mechanically?** In Rust, `unsafe` doesn't change
  what code executes — it lifts specific compiler-enforced guarantees (no dereferencing raw
  pointers, no calling unsafe fns, etc.) within the block. Does Asili's `wazi` work the same way
  (a lexical scope that relaxes semantic-analyzer checks — e.g. allows `Kiashiria` operations
  that would otherwise be rejected), or is it closer to a capability/permission gate at the
  function-call level (certain builtins simply refuse to be called outside a `wazi` context,
  checked at the call site rather than by relaxing static checks)? These have different
  implementation shapes: the first needs the semantic analyzer to track "are we inside `wazi`"
  as scope state (similar to how it already tracks `loop_depth` for `vunja`/`endelea`
  validity — a real, existing precedent in `core/parser/src/semantic/analyzer.rs`); the second
  is a simpler runtime check inside each gated builtin.
- **`Kiashiria` (raw pointer) representation.** The existing `Anuani` type
  (`core/evaluator/src/value/mod.rs`, `Value::Anuani(u64)` — "raw memory address, used by
  syscall and kiungo") is already a bare-address value type used by exactly the two stubs
  (`syscall`, `kiungo`) that `wazi`-gated code would plausibly use. Is `Kiashiria` meant to be a
  new, distinct type from `Anuani`, or is `Anuani` actually the spec's `Kiashiria` under a
  different name (the same naming-discrepancy pattern flagged for `sambamba`/`tenda` in
  [concurrency-async-design.md](concurrency-async-design.md) — worth checking deliberately
  rather than assuming)? If they're meant to be the same thing, `Anuani` should either be
  renamed or the spec updated to use it, rather than introducing a second overlapping type.
- **`Pini`/`Bafa<T>` remain platform-specific design work.** Embedded targets now exist:
  Cortex-M and ESP32 strict functions compile through `nguvu`, and `ubao` provides GPIO access on
  ESP32. The spec-named typed handles and `piga_pini` still need a decision on whether they add
  anything to the existing `ubao` interface.
- **`tenga_kumbukumbu` (manual heap allocation) and the existing allocator-failure model.**
  `07-execution-and-roadmap.md`'s "Embedded / strict mode" section already specifies that
  allocation failure should surface as `Tokeo<Tupu, Kosa>` in strict/embedded configurations —
  `tenga_kumbukumbu`'s return type needs to follow that same already-resolved convention
  (`08-resolved-decisions.md`'s allocation-failure rule) rather than introducing a separate
  error-handling convention for manual allocation specifically.

## Order of work, if this phase starts

1. Resolve the `Anuani`/`Kiashiria` naming question and the `syscall`-gating question — both
   cheap to settle now, expensive to leave ambiguous once real logic exists.
2. Decide the `wazi` mechanism (static relaxation vs. runtime capability check) — this is the
   actual architectural decision; it shapes whether `wazi` needs semantic-analyzer scope
   tracking or is purely a runtime concern.
3. `vuta_vifaani`/`anwani_ya`/`tenga_kumbukumbu` (the three hardware functions that don't need an
   embedded target to make sense — an accelerator, an address-of operator, and manual heap
   allocation are all meaningful on desktop/server) can be implemented once (1) and (2) are
   settled.
4. Revisit `Pini`/`Bafa<T>`/`piga_pini` against the working device targets and `ubao` API before
   adding a second GPIO surface.

## Cross-references

- [implementation-status.md](implementation-status.md) — Phase IV is partially implemented,
  including the ESP32 device path. The earlier `syscall`-without-`wazi`-gate concern is resolved.
- [concurrency-async-design.md](concurrency-async-design.md) — the sibling Phase IV doc with the
  same naming-discrepancy pattern (`sambamba`/`tenda`) that `Anuani`/`Kiashiria` should be
  checked against.
- [data-shapes-design.md](data-shapes-design.md) — the more-advanced sibling doc (type-level
  recognition already exists there; nothing comparable exists yet for this doc's types).
- [docs/spec/01-philosophy-and-edp.md](../spec/01-philosophy-and-edp.md) /
  [docs/spec/04-type-system.md](../spec/04-type-system.md#other-primitives) /
  [docs/spec/05-standard-library.md](../spec/05-standard-library.md#hardware--nguvu-inside-wazi)
  — normative `wazi`/hardware surface this doc is drawn from.
