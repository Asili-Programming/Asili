# Asili workspace — build, test, install
# Requires: cargo (Rust toolchain)

.PHONY: build test install

build:
	cargo build

build-release:
	cargo build --release

test:
	cargo test

# Install pata, pata-lsp and the standalone runner `tenda` to DESTDIR. `tenda` sits
# beside `pata` so `pata jenga --namna release` can build standalone executables from it. (default: no prefix; copy to ~/.local/bin if desired)
# Usage: make install [DESTDIR=~/.local/bin]
DESTDIR ?= /usr/local/bin
install: build-release
	install -d "$(DESTDIR)"
	install -m 755 target/release/pata-cli "$(DESTDIR)/pata"
	install -m 755 target/release/pata-lsp "$(DESTDIR)/pata-lsp"
	install -m 755 target/release/tenda "$(DESTDIR)/tenda"

# The VS Code extension lives in its own repository, Asili-Programming/asili-vscode; it builds its
# bundled pata-lsp from an Asili checkout. See that repository's README to install it.
