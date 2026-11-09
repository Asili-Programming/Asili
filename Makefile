# Asili workspace — build, test, install
# Requires: cargo (Rust toolchain)

.PHONY: build build-release runners test install
# `runners` copies one build aside before the next replaces it: keep the steps in order.
.NOTPARALLEL:

build:
	cargo build

build-release:
	cargo build --release

test:
	cargo test

# The two standalone runners in target/release: `tenda-msingi`, built without networking (for
# programs that use none: about a third the size), then `tenda`, the full one. The first build
# is copied aside before the second replaces target/release/tenda.
runners:
	cargo build --release -p asili-runner --no-default-features
	cp target/release/tenda target/release/tenda-msingi
	cargo build --release -p asili-runner

# Install pata, pata-lsp, pata-lint and the standalone runners `tenda` and `tenda-msingi` to
# DESTDIR. The runners sit beside `pata` so `pata jenga --namna release` can build standalone
# executables from them (the smaller one for a program that uses no networking).
# Usage: make install [DESTDIR=~/.local/bin]
DESTDIR ?= /usr/local/bin
install: runners build-release
	install -d "$(DESTDIR)"
	install -m 755 target/release/pata-cli "$(DESTDIR)/pata"
	install -m 755 target/release/pata-lsp "$(DESTDIR)/pata-lsp"
	install -m 755 target/release/pata-lint "$(DESTDIR)/pata-lint"
	install -m 755 target/release/tenda "$(DESTDIR)/tenda"
	install -m 755 target/release/tenda-msingi "$(DESTDIR)/tenda-msingi"
