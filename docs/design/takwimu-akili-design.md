# Takwimu/Safu (tensors/data) and Akili (NLP/LLM) design

**Status: not started. Phased below.** This doc was first written as a scoping note that deferred
phasing. It now proposes a phase order, lists every feature the spec names plus the ones they
need, and records what the code already provides. Sizes are rough (S/M/L/XL) and assume the
existing engine (`nguvu`, native-only, see [CLAUDE.md](../../CLAUDE.md)).

Normative description: [05-standard-library.md](../spec/05-standard-library.md) ("Moduli ya Takwimu
na Safu") and [06-tooling-and-ecosystem.md](../spec/06-tooling-and-ecosystem.md) ("Akili /
Takwimu"). Neither appears in the feature–phase map in
[07-execution-and-roadmap.md](../spec/07-execution-and-roadmap.md).

## What the code already provides

Checked against the code, not the spec:

| Need | Already there | Where |
|---|---|---|
| Scalar math (`exp` = `upeo_wa_e`, `tanh` = `tanjenti_h`, `kipeo`, `logi`, `mizizi`) | yes | `core/parser/src/builtins.rs` |
| Seeded random numbers (`nasibu`, `nasibu_mbegu`) | yes | `builtins/hisabati.rs` |
| Contiguous `f64`/narrow-int storage for `Orodha<Namba>` in native code | yes, **internal only** (`pub(crate)`) | `core/evaluator/src/numlist.rs` |
| Immutable byte buffer | `Baiti(Rc<[u8]>)` | `core/evaluator/src/value/mod.rs:44` |
| Byte encoding of unsigned integers | `baiti_ya_nambari` (1/2/4/8 bytes, `be`/`le`) | `builtins/msingi.rs` |
| Float ↔ bit pattern conversion | **no** | — |
| JSON read/write | `kutoka_json`, `kwa_json` | `builtins/json.rs` |
| Regex, hashes, base64 | `ruwaza_*`, `hashi_*`, `base64_*` | `builtins/ruwaza.rs`, `usimbaji.rs` |
| HTTP(S) client, TLS, custom headers | `http_ombi`, `tls_sanidi`, `vichwa` | `builtins/http_mteja.rs`, `builtins/mkondo.rs` |
| Environment variables (for API keys) | `pata_env` | `builtins/mfumo.rs` |
| Opaque values through native code | yes: native frames hold `Vec<Value>` (`host.rs:40`) | `core/evaluator/src/host.rs` |
| Opt-in modules (not ambient) | yes, `OPT_IN_MODULES` (currently `kasha_gc`) | `core/parser/src/builtins.rs:10` |
| Callable Asili functions from Rust | `call_by_name` (native host) | `core/evaluator/src/host.rs:309` |

Corrections to earlier claims in this doc and in the spec notes:

- **There is an HTTP client.** The earlier version said none existed. `http_ombi` and `tls_sanidi`
  are builtins. A hosted-LLM client therefore needs no new network stack.
- **The evaluator is not tree-walking.** The tree-walker was removed; the test suite refers to it
  only as a past reference. `CLAUDE.md` says native code is the only execution path.
- **Asili functions are not first-class values.** There is no lambda or closure in `Expr`
  (`core/parser/src/ast.rs`), and `Value` has no function variant. `safu.gradienti(kazi)` cannot
  receive a function as written. This needs a language decision (see D-2 below).
- **`umbo` is a keyword.** `safu.umbo(dim)` clashes with `KwUmbo` (`core/lexer`). Whether a keyword
  can follow `.` depends on `check_ident` (`core/parser/src/cursor.rs:83`), which was not verified
  by building. The name should be changed in the spec regardless.

## Decisions needed before implementation

These block the features that depend on them. Each choice is recorded below and must also go into
`docs/spec/08-resolved-decisions.md` before any stub is generated.

- **D-1 Location: in-tree, opt-in (decided).** Takwimu/Safu and Akili are modules gated by
  `leta safu` / `leta akili`, added to `OPT_IN_MODULES`. The heavy inference code sits behind a
  Cargo feature so the default build stays small. This matches the spec's "excluded for minimal
  embedded targets" rule.
- **D-2 `gradienti` mechanism: reverse-mode tape (decided).** Tensor ops record on a tape inside a
  `safu.rekodi` scope, and `safu.tofauti(loss, params)` returns gradients. No function argument
  and no language change are needed.
- **D-3 Element type: f32 from the start (decided).** Tensors store `f32` and arithmetic is `f32`.
  `Namba` stays `f64`, so every crossing between the two is an explicit conversion. Rust's `f32`
  arithmetic is IEEE and deterministic on native and wasm, provided no fused multiply-add is
  introduced.
  **Reduction accumulation: hybrid (decided).** Speed and precision together:
  1. Split the input into fixed-size chunks of 32 elements (chunk size is a spec constant).
  2. Sum each chunk in `f32` with a fixed pairwise tree, the shape fixed by the spec and not by the
     SIMD width. Vector code must follow the same tree, so SIMD and scalar code agree.
  3. Add the chunk partials in `f64`, then round once to `f32`.
  Pairwise summation keeps the error near O(log n) instead of O(n), and the `f64` combination
  removes the remaining drift across chunks. The `f32` inner loop stays vectorizable. Matmul uses
  the same rule along the inner dimension. Compensated (Kahan) summation is the fallback if the
  measured error is too high, at roughly 4× the cost of the chunk sum.
- **D-4 Naming: `safu.sura_ya` for the shape query (decided).** `umbo` is a keyword, so the spec's
  `safu.umbo(dim)` is replaced. `safu.zao_dot` stays, and `safu.geuza` becomes
  `safu.badili_mpangilio`. Settle in the spec before any stub is generated (`lib/std/*.asi` is
  generated).
- **D-5 Determinism: one fixed order everywhere (decided).** Reductions run in the same fixed
  order on native and wasm, so tensor results match bit for bit. Parallelism is allowed only where
  chunks are combined in that fixed order.

## Phased plan

### Phase 1: tensor foundation (P0)

- **1.1 `Tensor` value type** (L). A new `Value::Tensor(Rc<Tensor>)` where
  `Tensor { shape: Vec<usize>, data: Rc<[f32]> }` (D-3). Copy-on-write like `Orodha`
  (`Rc::make_mut`). Needs: `ValueType` and `format_value_type` entries, JSON and display
  handling, a `SendValue` variant (`Rc` → `Arc` for `sawia` threads), and a native-host
  pass-through test. Explicit `Namba` ↔ `f32` conversion builtins are part of this item.
  Follows D-5 for all reductions.
- **1.2 Module skeleton and stubs** (S). `safu` added to `BUILTIN_MODULES` and `OPT_IN_MODULES`;
  `lib/std/safu.asi` generated with `ASILI_GOLDEN=write cargo test -p pata-core stdlib_stubs`;
  LSP docs, Swahili diagnostics, `pata-lint` naming rules. Follows
  [sync-pata-toolchain](../../CLAUDE.md) rules.
- **1.3 Construction and shape** (S). `unda` (from `Orodha`), `sifuri` (zeros), `moja`
  (ones), `nasibu_umbo` (seeded random via `nasibu_mbegu`), and a shape query (named per D-4;
  not `umbo`, which is a keyword).
- **1.4 Elementwise, broadcasting, reductions** (M). `+ - * /` between tensors and scalars,
  `jumla` (sum), `wastani` (mean), `kubwa_zaidi`/`ndogo_zaidi` (max/min), with
  broadcasting rules written once and reused.
- **1.5 Reshape, transpose, index, slice** (M). `geuza`/`badili_mpangilio`, `kata` (slice),
  and `tensor[i, j]` indexing in the parser and evaluator. Index syntax needs a parser change
  and an `Expr` variant, which triggers [performance-guardrails](../../CLAUDE.md).
- **1.6 Tests** (M). Hand-checked golden results under `tests/golden/`, an `engines_agree`
  case, and wasm agreement.

### Phase 2: linear algebra and float bits (P0/P1)

- **2.1 Float bit conversion** (S). `namba_kwa_biti(x, upana)` and `biti_kwa_namba(b, upana)`
  for 4 and 8 bytes, plus 2 bytes for `f16`/`bf16`. Without these no weight file can be read or
  written, since `baiti_ya_nambari` encodes integers only.
- **2.2 Matrix multiply `zao_dot`** (M). Cache-blocked pure-Rust kernel, deterministic reduction
  order (D-5). Benchmark it against a naive triple loop and record the measured numbers.
  Not a `blas` binding: the repo keeps dependencies pure Rust and wasm-compatible.
- **2.3 Outer product, dot, matrix–vector** (S).
- **2.4 Normalize `kawaida`** (S). Mean/variance over a chosen axis; epsilon as an explicit
  argument.
- **2.5 Statistics `takwimu`** (S). Mean, variance, standard deviation, quantile, min/max over
  an axis.

### Phase 3: data (P1)

- **3.1 CSV reader and writer** (M). Hand-written RFC 4180 parser with quoted fields and embedded
  newlines. No `csv` crate. On `wasm32-unknown-unknown`, return an explicit `KOSA`, as `faili`
  does, rather than silently failing ([wasm-driver-design.md](wasm-driver-design.md)).
- **3.2 `pakia.jozi`** (S). Wrapper over `kutoka_json`, producing a `Tensor` from nested
  arrays.
- **3.3 `pakia.gawanya`** (S). Train/test split with a seeded shuffle through `nasibu_mbegu`.
  The result must be reproducible.
- **3.4 Batching** (S). A builtin that yields fixed-size batches of rows.
- **3.5 Binary tensor format `.safu`** (M). Magic bytes, version, dtype, shape, then the raw
  data. Versioned like `.asb` (`BYTECODE_VERSION`/`IMAGE_VERSION` rules apply).

### Phase 4: autodiff and training (P1, depends on D-2)

- **4.1 Reverse-mode tape** (L). Tensor ops record their inputs and backward closures on a tape
  while a `safu.rekodi` scope is active. `safu.tofauti(loss, params)` runs the backward pass.
  Each op's gradient is written once in Rust and covered by a test.
- **4.2 Gradient checker** (S). Finite-difference comparison for every op, run in `cargo test`.
- **4.3 Optimizers** (M). SGD and Adam, as Rust builtins that update tensors in place. Mutation
  here conflicts with copy-on-write and needs an explicit `safu.sasisha` (update) operation.
- **4.4 `f16`/`bf16` storage** (M). `f32` is already the element type (D-3), so this phase only adds
  the half-precision formats needed to read real weights. Conversion to `f32` happens at load time.

### Phase 5: Akili, hosted mode (P1, no tensor dependency)

- **5.1 Hosted client `akili.zungumza`** (M). Calls an LLM HTTP API through `http_ombi`, reads the
  key from `pata_env`, parses the JSON response. Returns `Tokeo<Neno, Neno>`. This is useful on
  its own and needs no tensor work.
- **5.2 Error and retry policy** (S). Uses the existing `jaribu_tena` and timeout options. Rate
  limits and retries are explicit, not hidden.
- **5.3 Swahili prompts and messages** (S). User-facing text follows
  [swahili-docs-and-errors](../../CLAUDE.md).

### Phase 6: Akili, local inference (P2, depends on phases 1–4)

- **6.1 Tokenizer** (M). Byte-level BPE, pure Rust, tested on Swahili text. Vocabulary loaded
  from a file. Not a dependency on the `tokenizers` crate unless D-1 allows it.
- **6.2 Weight loading** (M). `safetensors` (JSON header plus raw data) first, since it is the
  simplest format. Then GGUF. Needs 2.1 for `f16`/`bf16`.
- **6.3 Transformer kernels** (L). Embedding, RMSNorm or LayerNorm, RoPE, causal attention with a
  KV cache, softmax, sampling (greedy, temperature, top-p). Everything runs on `Tensor` ops.
- **6.4 Generation loop** (M). The `akili.zalisha` (generate) builtin: tokenize, run, sample,
  detokenize, stop tokens.

### Phase 7: accelerators (P3, optional)

- **7.1 `Bafa<T>` aligned buffer** (M). The buffer type from [wazi-hardware-design.md](wazi-hardware-design.md).
  Needed before any device transfer.
- **7.2 `vuta_vifaani` device offload** (XL). A GPU backend (for example `wgpu`) behind a Cargo
  feature. Not compatible with the wasm target without a separate path. Only worth doing after
  the CPU kernels in 2.2 and 6.3 are measured and found to be the limit.

### Not recommended in-tree: LLM training from scratch (P3, research)

Training a useful language model needs GPU or accelerator throughput and days of compute.
Phases 1–4 make training small models possible on CPU for tests and examples. Pretraining a
production-scale model is not a reasonable goal for this interpreter. Revisit once phase 7 exists
and there is a concrete user.

## Full ML/AI scope (the goal, not only the phases above)

The target is full coverage of the ML/AI space, not a subset. Phases 1 to 7 are the foundation.
Each tier below is tracked as issues on project 17. Priorities order the work; they do not drop
any tier. Issue ranges: tier 1 #105–#111, tier 2 #112–#115, tier 3 #116–#117, tier 4 #118–#121,
tier 5 #122–#125, tier 6 #126–#127, tier 7 #128–#129, tier 8 #130.

| Tier | Scope | Priority |
|---|---|---|
| 1 | Core ops: shape ops, conv and pooling, activations, losses, layers, attention, RoPE, autodiff for all of them, gradient checkpointing | P0 to P2 |
| 2 | Training: Adam and AdamW, schedules, clipping, accumulation, mixed precision, data pipeline, full checkpoints | P0 to P1 |
| 3 | Classical ML: linear and logistic regression, kNN, naive Bayes, k-means, PCA, trees, forests, SVM; metrics; cross-validation | P2 |
| 4 | Compute: SIMD (AVX2, NEON, simd128), multithreading with fixed-order reductions, GPU backend, fusion, memory pool, int8/int4 kernels | P1 to P2 |
| 5 | LLM: KV cache, paged attention, sampling, chat templates, LoRA/QLoRA, DPO/RLHF, speculative decoding | P1 to P2 |
| 6 | Distributed: data parallel all-reduce, model and pipeline parallelism, sharded optimizer, fault tolerance | P2 |
| 7 | Vision, audio, generative: image and audio I/O, augmentation, ViT, U-Net, diffusion | P2 |
| 8 | Tooling: experiment tracking, profiling, tensor inspector | P2 |

**Open decisions for these tiers:**
- **D-6 GPU backend: both from the start (decided).** `wgpu` (portable across vendors and
  browsers) and CUDA are maintained in parallel. Every GPU kernel needs a test on both backends,
  and results must match the CPU path within a documented tolerance, since GPU reductions may
  differ from the D-5 order. The CUDA toolkit is a build-time requirement only for the CUDA
  feature, so the default build does not need it.
- **D-7 Distributed transport: deferred (decided).** Tier 6 waits until single-machine training
  and the GPU backends are measured and working. When it starts, the default proposal still builds
  on `asili-mtandao`.

## Cross-cutting requirements

Every phase above, when it lands:

- Adds the builtin to `core/parser/src/builtins.rs` (`BUILTIN_MODULES`, the one source of truth),
  then regenerates `lib/std/*.asi`.
- Uses Rust implementations in `core/evaluator/src/builtins/*` only (native code calls them
  through `BuiltinTable`). No second implementation in the native backend.
- Adds a golden test and an `engines_agree` case. Wasm agreement is checked by
  `driver/wasm/tests/agree.sh`.
- Adds a CHANGELOG entry and updates the wiki pages it affects
  ([update-wiki](../../CLAUDE.md)).
- Keeps user-facing diagnostics in Swahili.

## Cross-references

- [implementation-status.md](implementation-status.md) — this subsystem is listed in the
  recommended-work section.
- [package-manager-design.md](package-manager-design.md) — relevant to D-1.
- [wazi-hardware-design.md](wazi-hardware-design.md) — `Bafa<T>` and `vuta_vifaani` (phase 7).
- [wasm-driver-design.md](wasm-driver-design.md) — file-I/O conventions for `pakia.*` on wasm.
- [docs/spec/05-standard-library.md](../spec/05-standard-library.md#moduli-ya-takwimu-na-safu-data),
  [docs/spec/06-tooling-and-ecosystem.md](../spec/06-tooling-and-ecosystem.md#akili--takwimu),
  [docs/spec/08-resolved-decisions.md](../spec/08-resolved-decisions.md) — normative sources; D-1
  through D-5 must be recorded here before implementation starts.
