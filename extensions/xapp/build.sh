#!/bin/bash
# Build step of the extension xapp (run by tools/mkext-install.sh in its
# Arch container, as root): check and compile the program `xapp`
# (source/, a Cargo workspace) and put it into the extension. Rust comes
# from the same Arch snapshot; --locked: exactly the crates of Cargo.lock.
# A format, lint or unit test failure stops the build.
#
# Usage: build.sh <extension tree> <Arch snapshot>
set -euo pipefail
root=${1:?tree missing}
pacman --config /tmp/pacman.conf --cachedir /cache --noconfirm --needed -Sy rust gcc >/dev/null
src=/src/extensions/xapp/source
export CARGO_HOME=/work/cargo-home CARGO_TARGET_DIR=/work/xapp-target
cd "$src"
cargo --version
cargo fmt --check
cargo clippy --all-targets --locked --quiet -- -D warnings
cargo test --locked --quiet
cargo build --release --locked --quiet
install -Dm0755 "$CARGO_TARGET_DIR/release/xapp" "$root/usr/bin/xapp"
rm -rf "$CARGO_TARGET_DIR"
