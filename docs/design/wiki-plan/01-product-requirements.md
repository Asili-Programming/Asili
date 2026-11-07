# Asili wiki — Product Requirements Document

As of 7 November 2026 · Victor Kaiuki

The wiki (`github.com/Asili-Programming/Asili/wiki`, 29 pages) should document every Pata tool and
every language and library feature, in Swahili and English, and prove it with a coverage check
instead of trusting memory. Each wiki page stays in two-way sync with its `docs/` counterpart, as
`CLAUDE.md` requires.

## Problem and goals

The wiki is broad (about 36,000 words across 29 pages) but has measurable gaps, checked on
7 November 2026 against the CLI's own help output and the source:

| Gap | Evidence |
| --- | --- |
| Tracing undocumented | `pata fuatilia` and the `--fuatilia` flag appear on no wiki page |
| Coverage undocumented for users | `jaribu --chanjo` appears only in Changelog and Production Readiness |
| Debugger undocumented for users | `pata-dap` appears only in Changelog, Architecture, Contributing, Production Readiness |
| Standalone formatter undocumented for users | `pata-fmt` appears only in Changelog, Architecture, Production Readiness |
| No playground page | "playground" appears only in Changelog, Architecture, Contributing |
| Two stdlib modules missing | `ruwaza` and `usimbaji` are in `BUILTIN_MODULE_NAMES` but on no wiki page |
| One LSP feature missing | Inlay hints are not mentioned on Editor Support |
| One REPL help topic missing | `namba-kuu-sahihi` (in `docs/repl/sw/`) is on no wiki page |
| English only | No Swahili page, while the primary audience is Swahili-speaking learners |

Already complete: all 8 lint rules (`LINT001`–`LINT301`) are on Formatting and Linting; every `pata`
subcommand except `fuatilia` is on CLI Reference.

Goals:

1. **Complete.** Every Pata command, flag, standalone tool, LSP capability, lint rule, stdlib module
   and REPL topic has a wiki page or section.
2. **Proven complete.** A CI check fails when the source gains something the wiki does not cover.
3. **Bilingual.** The learner-facing pages exist in Swahili; reference pages follow.
4. **Runnable.** Examples link to the playground (embed or share link) so readers run them.

## Audiences

| Audience | Needs | Success looks like |
| --- | --- | --- |
| Swahili-speaking learner | Getting started and the language tour in Swahili, runnable examples | Finishes the tour, running each example |
| Developer | Exact reference: every command, flag, module, function | Finds a flag in one search |
| Editor user | What VS Code and Mwalimu do, how to debug | Sets up debugging from the Debugging page |
| Contributor | Architecture, how to add a builtin or lint rule, how docs sync | Adds a feature and its docs in one pull request |

## Coverage matrix

What must exist; "Page" is the target page (new pages in bold).

| Feature set | Count | Page | Status |
| --- | --- | --- | --- |
| `pata` subcommands | 11 | CLI Reference | 10 of 11 (missing `fuatilia`) |
| Their flags | All in `pata <amri> --msaada` | CLI Reference | Partial (missing `--fuatilia`, `--chanjo` in user terms) |
| Standalone tools: `pata-lint`, `pata-fmt`, `pata-lsp`, `pata-dap`, `tenda` | 5 | CLI Reference (new section "Zana za pekee") | `pata-lint` only, on Formatting and Linting |
| Tracing (`--fuatilia`, `pata fuatilia`) | 1 feature | **Tracing** | Missing |
| Debugging (`pata-dap`, VS Code F5) | 1 feature | **Debugging** | Missing |
| Testing and coverage (`jaribu`, `#[jaribio]`, `--chanjo`) | 1 feature | **Testing** | Spread over CLI Reference |
| Playground | 1 | **Playground** | Missing |
| LSP capabilities | 17 | Editor Support | 16 of 17 (missing inlay hints) |
| VS Code commands and settings | All in the extension's `package.json` | Editor Support | Partial |
| Lint rules | 8 | Formatting and Linting | Complete |
| Stdlib modules | 13 | Standard Library (+ Networking, Concurrency) | 11 of 13 |
| REPL help topics | 17 in `docs/repl/sw/` | **REPL** | 16 of 17 |
| Build targets (`native`, `wasm`, `cortex-m`) | 3 | CLI Reference, Hardware and Data | To check per target |

## Scope

**v1:** close every gap in the table; four new pages (Tracing, Debugging, Testing, Playground) plus
a REPL page; the coverage check in CI; Swahili versions of Home, Getting Started, Language Tour,
Language Basics, Error Handling and Playground; a playground link on every runnable example.

**v2:** Swahili versions of all remaining pages; generated reference pages (CLI, stdlib) from
source; wiki content rendered on the website (website PRD FR-11).

**Out of scope:** replacing the GitHub wiki with another system in v1.

## Functional requirements

| ID | Requirement | Priority | Release |
| --- | --- | --- | --- |
| WK-1 | Every `pata` subcommand and every flag from `--msaada` is on CLI Reference | Must | v1 |
| WK-2 | The five standalone tools are documented with their flags | Must | v1 |
| WK-3 | New pages: Tracing, Debugging, Testing, Playground, REPL | Must | v1 |
| WK-4 | Every LSP capability and VS Code command and setting is on Editor Support | Must | v1 |
| WK-5 | Every stdlib module and function is on Standard Library or its topic page | Must | v1 |
| WK-6 | A coverage check in the Asili repo's CI fails when a command, flag, tool, module, lint rule, LSP capability or REPL topic is missing from the wiki | Must | v1 |
| WK-7 | Learner pages exist in Swahili, linked both ways with their English twin | Must | v1 |
| WK-8 | Every runnable example has a "Iendeshe kwenye uwanja" (run in playground) link | Should | v1 |
| WK-9 | Each page follows one template (summary, example, reference, see also) | Should | v1 |
| WK-10 | The sidebar is regrouped by task (learn, use the tools, reference, internals, project) | Should | v1 |
| WK-11 | CLI Reference and Standard Library generated from source | Could | v2 |

## Non-functional requirements

| ID | Area | Requirement |
| --- | --- | --- |
| WK-NFR-1 | Sync | Every wiki change has its `docs/` counterpart in the same pass, per `CLAUDE.md` and the `update-wiki` skill mapping |
| WK-NFR-2 | Accuracy | Every code example compiles with the current `pata`; checked in CI |
| WK-NFR-3 | Language | Swahili pages follow the project's `swahili-docs-and-errors` conventions; English stays the language of `docs/` (repo convention) |
| WK-NFR-4 | Links | No broken internal or external links (checked weekly) |
| WK-NFR-5 | Findability | Every feature reachable in two clicks from Home or the sidebar |

## Success metrics

| Metric | Source | Target |
| --- | --- | --- |
| Coverage check items passing | WK-6 CI job | 100% |
| Examples that compile | Example check in CI | 100% |
| Learner pages with a Swahili twin | Sidebar | 6 of 6 in v1 |
| Broken links | Link checker | 0 |

## Open questions

- [ ] Swahili page names: Swahili titles (`Anza`, `Ziara-ya-Lugha`) or a prefix (`sw-Getting-Started`)?
- [ ] Does the coverage check read the wiki repo directly in CI (second checkout) or a copy in
  `docs/`?
- [ ] Who reviews Swahili pages before they go live?
