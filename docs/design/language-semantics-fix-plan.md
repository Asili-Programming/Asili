# Language semantics: fix plan

Status: **plan, not started.** Written from the audit of the current `develop` tree (debug build,
probes run through `pata-cli jenga --tenda`). Every location below was read in the code; items
marked **decision** need an answer before implementation starts (see the last section).

Scope: the 11 defects in the semantics audit, plus the doc corrections they force. Ordered by
dependency, not by size.

---

## 0. Ground rules for every item

These come from the project's CLAUDE.md files and apply to each item below.

- **Tests first.** Each defect gets a failing test before the fix: a `core/parser/tests/`
  semantic test for analyzer changes, and an entry in `core/evaluator/tests/golden/engines_agree.txt`
  for runtime changes. Run `cargo test --workspace` before and after; record the baseline.
- **One semantics source.** Operators, casts and methods live in `eval/ops.rs`,
  `eval/methods.rs`, `host.rs::numeric_op`. Native code calls them. Do not re-implement a rule in
  `nguvu/`.
- **Engine checks** for any change under `core/evaluator/`: `engines_agree.rs`,
  `native_tiers.rs`, `tests/fuzz.rs`, `driver/wasm/tests/agree.sh`, and the Sudoku benchmark
  (`examples/sudoku/bench/run.sh`). Expected: solve and attempt count still exactly 90,665, speed
  at or below clang `-O2`.
- **Format versions.** Bump only what changes: `BYTECODE_VERSION` (`asb.rs:48`, now `"21"`),
  `IMAGE_VERSION` (`nguvu/mod.rs:330`, now `8`), `ABI_VERSION` (`aot.rs:27`, now `11`).
- **Swahili text.** Every new or changed diagnostic message goes through the
  `swahili-docs-and-errors` skill. Every new diagnostic code goes through `sync-pata-toolchain`
  (LSP, lint, docs).
- **Docs and wiki.** Spec files (`docs/spec/04-type-system.md`, `08-resolved-decisions.md`),
  `docs/language/` pages, `docs/design/implementation-status.md`, and the wiki via `update-wiki`.
  Spec changes also get a `docs/spec/CHANGELOG.md` bullet and a `CHANGELOG.md` entry under
  `[Unreleased]`.
- **Examples.** Items 3 and 4 can reject programs that currently compile. The CI `examples` job
  (`lint`, `jenga`, `jaribu` on every example) must stay green. 24 example files define
  `kazi kuu`, so an audit pass is part of those items.
- **Boards.** Each item gets an issue on project 16 (bug tracker), moved to In progress when it
  starts and Done only after it is merged.
- **Git.** One `git flow bugfix start` branch per item, no AI trailers, confirm before any push.

---

## 1. Where the code is now (map)

| Concern | File and line | Notes |
|---|---|---|
| Type enum for the analyzer | `core/parser/src/ast.rs:783` (`ValueType`) | No fixed-width variant. |
| Type string → `ValueType` | `core/parser/src/semantic/types.rs:47` | Maps `Biti*`/`uBiti*` to `Namba`. The TODO at the top of this file describes the problem. |
| Type display | `core/parser/src/ast.rs:213`, `:222`, `:738` | `format_value_type` and `Display for ValueType`. |
| `weka` declared-type check (SEM010) | `core/parser/src/semantic/analyzer.rs:597-615` | Calls `compatible(declared, inferred)`. |
| Structural compatibility | `analyzer.rs:2119` (`compatible`) | No arm for `Hamna` against `Chaguo`. |
| Literal `Hamna` typing | `analyzer.rs:1108` | `Expr::Hamna` → `ValueType::Hamna`. |
| Copy types | `analyzer.rs:2099` (`is_copy_type`) | Fixed list. No `Nakala` lookup. |
| Call argument checking | `analyzer.rs:1424-1440` | User-function arguments are checked with `UseMode::BorrowImm`. This is why moves don't happen. |
| Method calls | `analyzer.rs:1545-1610` | Receiver checked with `UseMode::BorrowImm` even for mutating methods. `method_receiver` (`:2275`) has no `Rejeo` arm. |
| Borrow state | `analyzer.rs:14`, `:691`, `:2013`, `:2030-2039` | `imm_borrows` is a counter that is never decremented on last use. |
| Mutating-method table | `core/parser/src/builtins.rs:929` (`MUTATING_METHODS`) | Exists and is used by the evaluator, but not by the analyzer. |
| Cast (analyzer) | `analyzer.rs:1407-1423` | Already returns `Chaguo<T>` for fallible targets. |
| Cast (runtime) | `core/evaluator/src/eval/methods.rs:1356` (`cast_value`) | `Biti*` branch at `:1472-1490` range-checks an `f64` and never truncates or converts. |
| Native cast opcode | `core/evaluator/src/host.rs:1005` | Calls `methods::cast_value`. |
| Value representation | `core/evaluator/src/value/mod.rs:127` | `Namba(f64)`, `Chaguo(Option<Box<Value>>)`, and a separate `Hamna` variant. |
| Bytecode static types | `core/evaluator/src/bytecode.rs:35` (`Ty`), `:55` (`from_type_name`) | `Num`, `Bool`, `List`, `Val`. Every fixed-width or `T?` type is `Val`. |
| Let lowering | `bytecode.rs:1486-1505` | Declared type selects the register file. No value coercion to the declared type. |
| `kama Neno` fallback | `methods.rs:1446-1468` | `format!("{v:?}")` for non-scalar values. |
| `chapisha` | `core/evaluator/src/builtins/matumizi.rs:20-26` | Prints only `Neno` arguments. |
| `.nakala()` | none | Not in any table. |
| SEM040 debug output | `pata/cli/src/pipeline/compile.rs:725-730` | Prints `context_map symbol={} created={:?} …` with English keys and Rust debug spans. |
| Diagnostic model | `core/diagnostics/src/lib.rs:56` (`ContextMap`) | `Serialize`. The LSP may read it, so keep the struct. |
| Builtin method table | `core/parser/src/builtins.rs` | Source of truth for builtin method names and signatures. |

---

## 2. Items

Sizes: **S** under a day, **M** a few days, **L** a week or more, counting tests and docs.

### Item 3: a value passed to a user function is moved (S, low risk, but examples may break)

**Defect.** `f(v)` then `v.urefu()` compiles and runs.

**Cause.** `analyzer.rs:1424-1440`: user-function arguments use `UseMode::BorrowImm`.

**Change.**
- In the `fn_map` branch, check each argument with the mode from its parameter type:
  `Rejeo`/`Rejeo_Tenda` parameters borrow; every other parameter moves. Read the mode from
  `type_from_decl(&param.ty.name)`.
- Arguments that are copy types (`Namba`, `Ukweli`, …) keep working because `is_copy_type` returns
  true.

**Engines.** No runtime change. Moves are analyzer-only.

**Tests.** Semantic: `f(v); v` → SEM040. `f(n); n` with a `Namba` still compiles. `f(azima v)` does
not move `v`. Golden: none needed.

**Risks.** Any example that reuses a value after passing it will now fail. Run `pata-cli lint`
and `jenga` over every example before merging, and fix each example rather than the rule.

**Docs.** `docs/spec/04` already says moves happen; add the diagnostic to the list in `docs/language`.

---

### Item 4: mutating a value while a borrow is still in use (L, the largest analyzer change)

**Defect.** `r = azima v; v.ongeza(3); r` compiles.

**Cause (two parts).**
1. `analyzer.rs:1555`: the receiver of every method call is checked with `UseMode::BorrowImm`,
   even when the method is in `MUTATING_METHODS`. A mutation is therefore never seen as a mutable
   borrow.
2. `imm_borrows` (`:2013`) is incremented and never decremented at last use. Turning on the
   borrow check without fixing this gives false positives: `r = azima v; use r; v.ongeza(3)`
   would fail even though `r` is dead.

**Change.**
- Part 1: in the method-call arm, if `method_name` is in
  `MUTATING_METHODS[method_receiver(&receiver_ty)]`, check the receiver with
  `UseMode::BorrowMut`. For user `shughuli` methods, use the first parameter's declared mode
  (`self` vs `Rejeo_Tenda`).
- Part 2: compute last use. Before analyzing a function body, walk it once and record, for each
  binding, the index of its last reference. A borrow is live only until that index. Replace the
  counter with a set of live borrows keyed by the reference's binding and span. This is the
  "re-enable immutable borrow check" TODO at `analyzer.rs:~2050` (Phase III).
- Keep `SEM041` (mutable while immutable live) and `SEM044` (move while mutable live) semantics.

**Engines.** None. This is analysis only.

**Tests.** `r = azima v; v.ongeza(3); r` → error. `r = azima v; r.urefu(); v.ongeza(3)` → OK
(last use before mutation). Loops: a borrow inside a loop body that is used on the next
iteration. Struct method with `self` by `Rejeo_Tenda`. The existing `azima_tenda`+`azima` test
must still give SEM041.

**Risks.** This is the riskiest analyzer change. Liveness across `ikiwa`, `wakati`, `kwa` and early
`rejesha` needs a dedicated test set. Ship behind a diagnostic that can be turned off by a
feature flag until the examples pass.

**Decision D4** (below): last-use liveness vs. lexical scopes.

---

### Item 5: methods and fields on a borrow (S)

**Defect.** `r = azima v; r.urefu()` → SEM039 `aina 'Rejeo<…>' haina njia`.

**Cause.** `method_receiver` (`analyzer.rs:2275`) and the method-call arm do not unwrap `Rejeo`.
Field access (`r.jina`) has the same gap and must be fixed together.

**Change.**
- At the top of method-call and field-access checks, replace `Rejeo(t, _)` with `t` for lookup
  only. The receiver keeps its borrow mode.
- `compatible` (`:2119`) already compares `Rejeo` structurally. Check that `chapisha(r)`
  (argument type `Neno` against `Rejeo<Neno>`) is accepted or rejected consistently.

**Engines.** None. `UnaryOp::BorrowImm` is a no-op at runtime (`eval/ops.rs:215`,
`bytecode.rs:2338`).

**Tests.** Semantic: `r.urefu()`, `r.jina`, `r.ongeza(...)` through `azima_tenda`. Golden: a
program that calls methods through a borrow and prints the result.

---

### Item 6: nullable annotations (M, needs a representation fix first)

**Defect.** `weka x: Namba? = Hamna`, `weka x: Chaguo<Namba> = Hamna` and `weka x: Namba? = 4` are
all SEM010. The spec's own example is the first one.

**Cause (two parts).**
1. Analyzer: `Expr::Hamna` is typed `ValueType::Hamna` (`analyzer.rs:1108`). `compatible`
   (`:2119`) has no `(Chaguo(_), Hamna)` arm. `Chaguo::Hamna` works only because of the
   `Struct("Chaguo")` arm.
2. Runtime: two representations of "empty" exist. `Value::Hamna` (`value/mod.rs:138`) and
   `Value::Chaguo(None)`. `pattern.rs:24` accepts both, which hides the problem. Pick one and make
   every producer use it.

**Change.**
- Analyzer: `(Chaguo(_), Hamna) → true`. Literal `Hamna` gets its type from the expected type when
  there is one.
- Runtime: normalize to `Value::Chaguo(None)` (the form `.pata()` already returns). Make
  `StoredConstant::Hamna` and `Expr::Hamna` in `bytecode.rs:829` produce it. Remove the `Value::Hamna`
  variant only after the grep shows no other producers; otherwise keep it and make it an alias.
- If decision D1 is "yes": at `Let` lowering (`bytecode.rs:1486`), when the declared type is
  `T?` and the value's inferred type is `T`, emit a wrap into `Chaguo(Some(_))`. Analyzer side:
  `compatible(Chaguo(t), u)` → `compatible(t, u)`.

**Engines.** Yes: the `Let` wrap is a bytecode change, so `BYTECODE_VERSION` bumps.

**Tests.** Semantic: the three declarations above, and `weka x: Namba? = "a"` still fails. Golden:
`Hamna` and `Chaguo::Hamna` print and match identically.

**Docs.** `docs/language/09` (`: Chaguo<Namba>` on a bare `Hamna`) becomes true; leave it and add the
test. `docs/spec/08` §9.7 states `T?` and `Chaguo<T>` are one type, so no spec change is needed
unless D1 changes it.

---

### Item 7: `.nakala()` (S)

**Defect.** `v.nakala()` → SEM040.

**Cause.** Not implemented anywhere (no table entry, no runtime arm).

**Change.**
- Add `nakala` to `PURE_METHODS` for every receiver kind that can be cloned: `Neno`, `Orodha`,
  `Kamusi`, `Seti`, `Chaguo`, `Tokeo`, `Jozi`, `Struct`, `Baiti`. Reject it for `Faili`, `Mkondo`,
  `KashaGC` unless D5 says otherwise.
- Runtime: a single arm in `eval/methods.rs` that returns `v.clone()`. For `Orodha`, `Value::clone`
  is an `Rc` bump and writes copy on write (`Rc::make_mut`), so the result is independent. Check
  `Kamusi` and `Seti` the same way.
- Analyzer: the method's return type is the receiver type.

**Engines.** Method added to the shared table, so native code gets it through `methods::call`.
Check `native.rs`'s method metadata (`num_reads`/`num_writes`) if the method is in a list fast path.

**Tests.** Semantic: `v.nakala()` then `v` still usable. Golden: mutate the copy, original unchanged.

---

### Item 8: `Nakala` trait (M, depends on item 7)

**Defect.** The trait does not exist. Copy is a fixed list in `is_copy_type` (`analyzer.rs:2099`).

**Change.**
- Seed `Nakala` as a built-in trait with no methods, the same way `Inasomeka`/`Inandikika` are
  seeded (`implementation-status.md`, Sifa entry).
- `shughuli ya Umbo kwa Nakala {}` marks the struct as copy. `is_copy_type` consults a set of
  user types that implement it.
- The analyzer checks that every field of a `Nakala` struct is itself copy (a `Nakala` struct
  holding a `Neno` is an error). New diagnostic code.
- Runtime: nothing. Copies of structs are `Rc` bumps already.

**Engines.** None beyond the analyzer.

**Tests.** Semantic: `Nakala` struct reused after assignment → OK; struct holding `Orodha` marked
`Nakala` → error.

**Decision D5** covers `Kasha_GC` and handles.

---

### Item 9: `Chaguo` (and other values) printed as Rust debug text (S code, M docs)

**Defect.** `x kama Neno` where `x: Chaguo` prints `Chaguo(Hamna)` or `Chaguo(Kuna(Namba(3.9)))`.
The spec says conversion goes through `Onyesheka` (`docs/spec/04`).

**Cause.** `methods.rs:1446-1468`: the `Neno` branch falls back to `format!("{v:?}")` for any
value that is not a listed scalar.

**Change.**
- Add one function `display_value(&Value) -> String` next to `format_namba` in `value/mod.rs`. It is
  the only place that defines user-visible text for values. Scalars keep their current text.
  `Chaguo` renders as `Kuna(<inner>)` / `Hamna`. `Struct` renders as `Name { field: …, … }`.
  `Orodha` and `Kamusi` keep their current debug text if D3 says so.
- Use it in the `Neno` branch of `cast_value` and nowhere else.
- `chapisha` stays `Neno`-only (a deliberate choice). Add a clearer analyzer error for non-`Neno`
  arguments that suggests `kama Neno`.

**Engines.** Both engines call the same function, so they agree by construction. The golden file
(`engines_agree.txt`) contains one `Chaguo(` line, which changes.

**Tests.** Golden: every `Chaguo` and `Struct` case rendered. Semantic: the `chapisha` hint.

**Docs.** `docs/language/08` line 46 and surrounding examples document the debug form. They must
change with the code. The wiki page for the same topic changes through `update-wiki`.

**Decision D3** (below).

---

### Item 10: the SEM040 message leaks Rust debug output (S)

**Defect.** Output contains
`context_map symbol=s created=Some(Span { line: 3, column: 1 }) moved=…`.

**Cause.** `pata/cli/src/pipeline/compile.rs:725-730` formats the `ContextMap` with `{:?}`.

**Change.**
- Render the context as Swahili lines in `compile.rs`:
  `  ilitangazwa mstari {l}:{c}, ilihamishwa mstari {l}:{c}`, with the symbol name in the message.
  Keep `ContextMap` as is, because it is serialized for the LSP.
- Use the `swahili-docs-and-errors` skill for the wording. Check the same rendering in the LSP
  (`pata/lsp`) and the JSON output.

**Engines.** None.

**Tests.** A CLI test (`pata/cli/tests` or the `test_support` helper) that pins the rendered text.

---

### Item 1: fixed-width integer types (L, the largest item)

**Defect.** `Biti8` 100 + 100 → `200`. `7 / 2` as `Biti32` → `3.5`. `Biti8 = 200` is accepted.
Overflow is not wrapped, and integer division is float division.

**Cause.** `types.rs:47` collapses `Biti*`/`uBiti*` into `Namba` at parse time. The runtime cast
(`methods.rs:1472-1490`) checks range on an `f64` and keeps the `f64`. `Ty::from_type_name`
(`bytecode.rs:55`) makes every `Biti*` a generic `Val`.

**Change, in layers.**

1. **Analyzer types.** Add `ValueType::Biti { bits: u8, signed: bool }` (or one variant per
   width). Map it in `types.rs:47`, `format_value_type`, and `Display`.
2. **Typing rules.**
   - `Biti op Biti` of the same width → that width. Different widths → SEM error.
   - `Biti op Namba literal` → the literal is checked against the width at compile time. A
     literal out of range is a new diagnostic (for item 1's #11).
   - `Biti op Namba` (non-literal) → error, so the programmer casts explicitly.
   - Comparisons → `Ukweli`.
   - Division `/`: truncates toward zero. Division by zero: **decision D6**.
   - `%`: remainder with the sign of the dividend (D7).
3. **Runtime value.** Add `Value::Biti(i128, Width)` where `Width` is an enum of the six widths.
   `i128` holds every `Biti64`/`uBiti64` value exactly. One helper, `wrap(width, i128) -> i128`,
   in `eval/ops.rs`, applies two's-complement wrapping. Both engines call it.
4. **Operators.** Arms in `eval/ops.rs` for the new value, through the existing `numeric_op`
   dispatch in `host.rs`. Native code never implements the rule itself.
5. **Casts.** Rewrite the `Biti*` branch of `cast_value` (`methods.rs:1472`):
   - from `Namba`: if finite, truncate toward zero, range-check, return `Chaguo(Kuna(Biti))` or
     `Chaguo(None)`.
   - from `Biti` of another width: range-check, same result shape.
   - from `Neno`: parse, range-check.
   This is the fallible cast the spec describes (`04` Casting).
6. **Bytecode.** Phase A: `Ty::from_type_name` keeps `Biti*` as `Val`, which is correct and
   simple. Phase B (optional, for speed): a `Ty::Int(width)` register file lowered only where
   `native::analyze_numbers` proves the range. Phase B is not needed for correctness and must not
   block phase A.
7. **Formats.** `BYTECODE_VERSION` bump (new `Value` variant reaches `.asb` through constants).
   `IMAGE_VERSION` and `ABI_VERSION` bump only if the native layer changes (phase B).
8. **Serialization.** `value/json.rs`: a `Biti` encodes as a JSON number and decodes back to the
   declared width. Add the round-trip test.
9. **Toolchain.** `core/parser/src/builtins.rs` signatures that mention `Biti` (`baiti_ya_nambari`,
   etc.) and the generated `lib/std` stubs (`ASILI_GOLDEN=write cargo test -p pata-core
   stdlib_stubs`). LSP hover uses the same table.

**Engines.** Yes, all of them: evaluator, native host, wasm. Expected sudoku numbers do not change
(the benchmark uses `Namba`); re-run to confirm.

**Tests.** Semantic: literal range, mixed-width error, comparison result type. Golden: wrap at
both ends (`Biti8` 127+1 → -128, `uBiti8` 0-1 → 255), `Biti64` overflow, casts that succeed and
fail, division truncation and the D6 case. Fuzz: wrapping equality between optimized and
unoptimized native code.

**Risks.** The analyzer's binary-op typing currently treats every numeric as `Namba`. Changing it
affects every example using `Biti*`. Grep `examples/` and `docs/` for `Biti` before starting to
size the migration.

---

### Item 2: the fallible cast returns the wrong type (folded into item 1)

**Defect.** `3.9 kama Biti32` → `Chaguo(Kuna(Namba(3.9)))`.

**Change.** Covered by item 1, step 5. Listed separately because it is a visible symptom with its
own test.

---

### Item 11: out-of-range literals accepted (folded into item 1)

**Defect.** `weka a: Biti8 = 200` runs.

**Change.** Covered by item 1, step 2: a compile-time check of literals against the declared width,
with a new diagnostic. Runtime casts already return `Hamna`, which is correct.

---

## 3. Order of work

Each wave can run in parallel within itself. Later waves depend on earlier ones as noted.

| Wave | Items | Why this order | Size |
|---|---|---|---|
| 0 | Test baseline, `cargo test --workspace`, Sudoku baseline | Need to know the current state before changing it | S |
| 1 | 10 (diagnostic text), 5 (borrow method lookup), 3 (move on call) | Small and independent. Item 3 needs the example audit. | S each |
| 2 | 7 (`.nakala()`) | Needed by item 8 | S |
| 3 | 6 (nullable), 9 (display) | Both touch `Chaguo` runtime form. Do 6's representation fix before 9. | M |
| 4 | 4 (borrow mutation, liveness) | Depends on 5 (method receiver logic). Largest analyzer change. | L |
| 5 | 1, 2, 11 (fixed-width) | Biggest engine change. Needs D2 and D6 first. Independent of 4. | L |
| 6 | 8 (`Nakala` trait) | Depends on 7 | M |

Docs and wiki are done per wave, not at the end, so the spec and the wiki never disagree for long.
The stale `implementation-status.md` entries (trait dispatch, unknown builtin method, decimal
literals, the "recommended order") are corrected in wave 0, because they are already wrong.

---

## 4. Verification gate per item

Before a branch is merged:

1. The new semantic and golden tests fail on the old code and pass on the new.
2. `cargo test --workspace` passes. `cargo clippy -p asili-evaluator --lib` passes.
3. `pata-lint`, `jenga` and `jaribu` pass for every example (the CI `examples` job).
4. For evaluator changes: `engines_agree`, `native_tiers`, `fuzz` (with the CI settings),
   `driver/wasm/tests/agree.sh`, and `run.sh` (90,665 exact, speed at or below clang `-O2`).
5. `CHANGELOG.md` and, for spec changes, `docs/spec/CHANGELOG.md`.
6. The wiki and `docs/` are in sync (`update-wiki`).
7. The project board item is moved to Done only after merge.

---

## 5. Decisions needed

Each of these changes what gets built. My recommendation is in bold.

- **D1. Implicit wrap into `T?`.** Should `weka x: Namba? = 4` store `Kuna(4)`?
  **Yes**, so `T?` is usable without writing `Chaguo::Kuna`. Without it, only `Hamna` and explicit
  `Chaguo::Kuna(...)` work, which is awkward.
- **D2. Fixed-width representation.** `Value::Biti(i128, Width)` boxed first (**recommended**), or
  keep `f64` and wrap after each operation. The `f64` route cannot represent `Biti64`/`uBiti64`
  exactly, so it is not recommended.
- **D3. Display of `Chaguo` and other values.** Swahili form `Kuna(3)` / `Hamna` (**recommended**),
  or keep Rust debug text and document it. The spec says `Onyesheka`, so the Swahili form matches
  the spec.
- **D4. Borrow model.** Last-use (non-lexical) liveness (**recommended**; the spec's "lifetimes are
  inferred" wording points here), or scope-based borrows (simpler, rejects valid programs).
- **D5. `.nakala()` on handles.** Reject for `Faili`, `Mkondo`, `Kasha_GC` (**recommended**, the
  spec says those must not be silently copied), or share the handle.
- **D6. `Biti / 0`.** Runtime error that halts with a Swahili message (**recommended**; it matches
  `paparika`), or return `Hamna` (changes the result type from `Biti` to `Chaguo<Biti>` only for
  division, which is confusing).
- **D7. `%` with negative operands.** Keep the current result (`-7 % 3` = `-1`, sign of dividend,
  **recommended**), and document it in `docs/spec/04`.
- **D8. `kwa i kutoka a hadi b`.** The upper bound is excluded today (`0..3` gives 0, 1, 2). The
  docs use it that way. **Keep it**, and state it in `docs/spec/03`. The audit did not find a
  contradicting example.

---

## 6. What this plan does not cover

- Trait objects / dynamic dispatch, first-class functions, closures. Listed as not implemented in
  the spec. Not defects.
- Phase III items (`Mfululizo`, lifetime inference for returned references, `SEM120`). Item 4
  covers the local borrow check only.
- `syscall`/`kiungo` gating and the FFI stub. Separate decision, tracked in
  `implementation-status.md`.
- Items not probed in the audit: `?` propagation, `Namba_Kuu`/`Namba_Sahihi` arithmetic, `Seti`,
  `Baiti`, the concurrency rules. Each needs its own audit pass before it is called done.
