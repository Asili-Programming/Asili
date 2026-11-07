# `pata kagua`

Purpose: lint `.as` sources with the `pata-lint` rules (`LINT001`–`LINT301`).

Inputs:
- optional path: one file or a directory (default: the current directory). A directory is walked
  recursively, skipping `kilele/` and hidden directories; `.asi` interface stubs are not linted
- optional `--makosa-tu`: report errors only, hiding lint-stage (`ukaguzi`) warnings
- rule settings from the nearest ancestor `pata.toml`'s `[lint.rules]` (per-rule severity,
  `severity = "ignore"` to disable, LINT101's `line_limit`)

Success:
- prints each finding as `<file>:<line>:<column> (<code>): <message>`, or
  `✓ <file> - hakuna tatizo lililopatikana` for a clean file; exits 0 when nothing was reported

Failures:
- a file that cannot be read or parsed: reported, the rest still linted, exit 1
- any finding (without `--makosa-tu`): exit 1
- an unknown flag, more than one path, or a path that is neither a file nor a directory: exit 2
