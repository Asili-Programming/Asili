# Asili wiki — Design Brief

As of 7 November 2026 · Victor Kaiuki

GitHub renders the wiki, so design here means structure, words and conventions, not colours. The
goal is that every page reads the same way and every example can be run.

## Voice

- Swahili pages: the register of the compiler's own messages; direct, polite, plain. Keywords stay
  in code formatting and are never translated (`kazi`, `weka`, `ikiwa`).
- English pages: plain, short sentences; numbers instead of adjectives.
- Both: second person ("you run", "unaendesha"), present tense, no exclamation marks, no emoji.

## Page template

```markdown
# Tracing

Follow what the compiler and your program did, step by step, as a tree or as JSON.

[Soma kwa Kiswahili](Ufuatiliaji)

## Example

    pata jenga --tenda --fuatilia

[Iendeshe kwenye uwanja](https://asili-programming.github.io/playground/?sample=...)

## How it works
...

## Reference

| Flag | Values | Default | Meaning |
| --- | --- | --- | --- |
| `--fuatilia` | `mti`, `json`, `json:<faili>` | `mti` | ... |

## See also

- [CLI Reference](CLI-Reference#jenga) · source: `docs/design/tracing.md`
```

## Conventions

| Element | Convention |
| --- | --- |
| Page titles | Swahili pages: Swahili title; English pages: English title; both name the Asili term in code if there is one |
| Commands | Always the full command in code: `pata jaribu --chanjo`, never "the coverage flag" alone |
| Flags tables | Columns: Flag · Values · Default · Meaning, in the order `--msaada` prints them |
| Diagnostics | Quoted exactly as the compiler prints them, Swahili text and code |
| Examples | Smallest complete program; runnable unless marked `asili,ignore` |
| Playground links | "Iendeshe kwenye uwanja" on Swahili pages, "Run in the playground" on English pages |
| Diagrams | Mermaid, top-down for flows, left-to-right for pipelines; at most 12 boxes |
| Callouts | GitHub alerts (`> [!NOTE]`, `> [!WARNING]`) only for "not yet supported" and "breaking" |
| Status of a feature | One line at the top of the section: "Inapatikana" / "Available", or "Bado" / "Not yet" with the roadmap link |

## Sidebar

- Five groups (Jifunze, Zana, Marejeo, Ndani, Mradi) with the English word beside the Swahili one.
- A language line at the very top: "Kiswahili · English".
- No more than 10 links per group; deeper pages are linked from their group's main page.

## Home page

- First line: what Asili is, one sentence, Swahili then English.
- Three buttons as links: Jaribu kwenye uwanja (playground), Anza (Getting Started), Zana
  (CLI Reference).
- A short "what is documented" table generated from the coverage check, so readers see the wiki is
  complete.

## Words for new pages

Drafts for native-speaker review:

| English page | Swahili page title |
| --- | --- |
| Getting Started | Anza |
| Language Tour | Ziara ya Lugha |
| Language Basics | Misingi ya Lugha |
| Error Handling | Kushughulikia Makosa |
| Playground | Uwanja |
| Tracing | Ufuatiliaji |
| Debugging | Kutatua Hitilafu |
| Testing | Majaribio |
