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

# The standalone runners in target/release, each built with fewer optional features (see
# `asili_evaluator::uwezo`) and copied aside before the next build replaces target/release/tenda:
# `tenda-ndogo` (no networking, no regular expressions), `tenda-msingi` (no networking), then
# `tenda` (everything). `pata jenga --namna release` uses the smallest that covers a program.
runners:
	cargo build --release -p asili-runner --no-default-features
	cp target/release/tenda target/release/tenda-ndogo
	cargo build --release -p asili-runner --no-default-features --features ruwaza
	cp target/release/tenda target/release/tenda-msingi
	cargo build --release -p asili-runner

# Install pata, pata-lsp, pata-lint and the standalone runners to DESTDIR. The runners sit beside
# `pata` so `pata jenga --namna release` can build standalone executables from them (the smallest
# that covers each program).
# Usage: make install [DESTDIR=~/.local/bin]
DESTDIR ?= /usr/local/bin
install: runners build-release
	install -d "$(DESTDIR)"
	install -m 755 target/release/pata-cli "$(DESTDIR)/pata"
	install -m 755 target/release/pata-lsp "$(DESTDIR)/pata-lsp"
	install -m 755 target/release/pata-lint "$(DESTDIR)/pata-lint"
	install -m 755 target/release/tenda "$(DESTDIR)/tenda"
	install -m 755 target/release/tenda-msingi "$(DESTDIR)/tenda-msingi"
	install -m 755 target/release/tenda-ndogo "$(DESTDIR)/tenda-ndogo"
