# Asili wiki — Technical Requirements Document

As of 7 November 2026 · Victor Kaiuki

The wiki stays a GitHub wiki (the `Asili.wiki.git` repo of Markdown pages) kept in two-way sync with
`docs/`. What changes is that completeness is checked by a program reading the same sources the
tools use, and reference sections are generated where a single source already exists. This document
implements the [Product Requirements Document](01-product-requirements.md) (WK-n).

## Sources of truth

Each fact the wiki states has one home in the code (the repo's write-once rule). The coverage check
reads these, never a hand-kept list:

| Fact | Single source | How the check reads it |
| --- | --- | --- |
| `pata` subcommands and flags | Each command's `--msaada` text | Run `pata --msaada` and `pata <amri> --msaada`; parse lines starting with `--` |
| Standalone tools and flags | `[[bin]]` in `pata/*/Cargo.toml`; each tool's `--help` | Read the manifests, run `--help` |
| Stdlib modules and functions | `core/parser/src/builtins.rs::BUILTIN_MODULES` | Small Rust test in `pata-core`, like the existing `stdlib_stubs` test |
| Lint rules | Rule codes in `pata/lint/src/rules/*.rs` | Same Rust test, via the lint crate's rule list |
| LSP capabilities | `ServerCapabilities` built in `pata/lsp` | Rust test calling the capabilities builder |
| REPL help topics | `docs/repl/sw/*.md` file names | File listing |
| VS Code commands and settings | `contributes` in `asili-vscode/package.json` | Optional; read when the extension repo is checked out beside Asili |

**Prerequisite in the CLI (done):** every command answers `--msaada`, even outside a project
(Asili `776dcaf`), and a `pata-cli` test checks every command listed in `pata --msaada`.

## The coverage check

- A test in the Asili repo, `pata/core/tests/wiki_coverage.rs`, next to `editor_word_lists.rs`, in
  the same style: it looks for the wiki checkout at `../Asili.wiki` or `ASILI_WIKI_DIR`, and skips
  with a notice when it is absent, so local `cargo test` never fails for want of a clone.
- In CI, a job checks out `Asili-Programming/Asili.wiki` beside the repo and runs the test with the
  directory set, so there it is enforced.
- For each item it searches the target page named in the PRD's coverage matrix for the exact token
  (`--chanjo`, `LINT203`, `ruwaza`, `inlayHint` or its documented name). A missing token fails with
  the item and the page it should be on.
- An allow-list file in the test names items deliberately undocumented (for example internal
  flags), each with a reason.

## Generated sections (v2)

Where a single source exists, generate the reference instead of writing it:

- CLI Reference: one section per command from `--msaada`, between `<!-- generated:cli -->` markers;
  hand-written prose stays outside the markers.
- Standard Library: function signatures and one-line descriptions from `BUILTIN_MODULES`, the same
  data `render_stub` writes into `lib/std/*.asi`.
- Regenerated with `ASILI_GOLDEN=write cargo test -p pata-core wiki_coverage`, matching how stubs and
  editor word lists are regenerated.

## Pages and naming

- New pages: `Tracing.md`, `Debugging.md`, `Testing.md`, `Playground.md`, `REPL.md`, each with a
  `docs/` counterpart added to the `update-wiki` skill's mapping table (for example
  `docs/design/tracing.md` → Tracing, `docs/howto/01-run-tests.md` → Testing, a new `docs/howto/`
  page → Debugging).
- Swahili pages: one file per page, named per the PRD open question; each starts with a link to its
  English twin and the reverse. `_Sidebar.md` gets a language line at the top.
- Mermaid diagrams render in GitHub wikis; use them instead of images where a diagram is needed.

## Runnable examples

- Examples in fenced `asili` blocks.
- A link under each runnable example to the playground: `/playground/?sample=<id>` for gallery
  samples, or a share link (`#k=…`) for one-off code, produced by a script so links never go stale
  by hand.
- The example check compiles every `asili` block from the wiki pages with `pata jenga` from the same
  CI job (blocks marked `asili,ignore` are skipped, for deliberately wrong code).

## Sync with `docs/`

- Unchanged rule: every wiki page maps to a `docs/` file (mapping in the `update-wiki` skill); a
  change to one updates the other in the same pass; the wiki push is confirmed with the maintainer.
- The coverage check runs against the wiki, but a gap is fixed in both places.

## CI summary

| Job | Runs | Fails when |
| --- | --- | --- |
| Wiki coverage | Pull requests and weekly, with a wiki checkout | Any item in the matrix is missing |
| Wiki examples | Same | An `asili` block does not compile |
| Wiki links | Weekly | A link returns 4xx or 5xx, or an internal page is missing |
