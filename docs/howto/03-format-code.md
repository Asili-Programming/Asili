# Formatting code

`pata nadhifu` auto-formats `.as`/`.asi` source files.

## Basic usage

From your project directory (where `pata.toml` lives):

```bash
pata nadhifu
```

Formats every `.as`/`.asi` file under the project in place and prints a summary:

```
imekamilika: mafaili 1 yalikaguliwa, 1 yamebadilishwa
```

## Check mode (for CI)

```bash
pata nadhifu --kagua
```

Doesn't write anything — exits 1 if any file isn't already in canonical form (and prints
`format haijafuata viwango: mafaili N/M`), exits 0 with `sawa: mafaili yote N yako katika
muundo sahihi` if everything's already formatted. Use this in CI instead of the write mode.

## Diff mode and JSON

```bash
pata nadhifu --diff          # never writes; prints a unified diff of what would change
pata nadhifu --kagua --json  # {sawa, jumla, yamebadilishwa: [paths]} instead of text
```

`--diff` exits like `--kagua` (1 if any file would change). `--json` works with `--kagua` or the
default write mode.

## Scoping to a path

```bash
pata nadhifu src/kuu.as
pata nadhifu src/
```

Only one path argument is accepted (`tolea njia moja tu` if you pass more than one); omit it
to format the whole project from the current directory.

## How it works

`pata nadhifu` is a **token-stream pretty-printer, not a full AST-based formatter** — it
re-tokenizes the source (capturing comments as trivia, since they aren't part of the AST) and
re-emits tokens with layout rules driven by token kind and brace/paren nesting depth, rather than
parsing and re-printing from the AST directly (AST nodes only carry a start line/column today,
not an end span, so span-slicing the source isn't viable yet). It never rewrites token contents,
so string/char literals containing `{`/`}`/`,` survive unchanged, and comments are preserved in
place rather than discarded. On a lex error (e.g. an unterminated string) it returns the input
unchanged rather than producing a mangled partial rewrite — safe to run on in-progress,
possibly-invalid source. See [nadhifu-formatter-design.md](../design/nadhifu-formatter-design.md)
for the full design, and [06-tooling-and-ecosystem.md](../spec/06-tooling-and-ecosystem.md) for
the exact formatting rules.

## Before committing

`pata thibitisha` (see [04-validate-docs.md](04-validate-docs.md)) requires the project to
already be in canonical format — run `pata nadhifu` first if `thibitisha` fails with `nadhifu
check imefeli`.

## Linting

`pata kagua` runs the linter (`LINT001`–`LINT301`, the `pata-lint` rules) over `.as` files:

```bash
pata kagua                 # the current directory, recursively (skips kilele/ and hidden dirs)
pata kagua src/kuu.as      # one file
pata kagua --makosa-tu src/  # errors only, hiding lint-stage warnings
```

Each finding prints as `<file>:<line>:<column> (<code>): <message>`; a clean file prints
`✓ <file> - hakuna tatizo lililopatikana`. `.asi` interface stubs are not linted. It exits 1 when
any file can't be read or parsed, and — without `--makosa-tu` — when any finding is reported.
Rules are configured per project in `pata.toml`'s `[lint.rules]` (severity, `severity = "ignore"`
to disable a rule, LINT101's `line_limit`). Mwalimu runs the same rules on every edit.
