#!/bin/bash
# Build step of the extension xapp (run by tools/mkext-install.sh in its
# Arch container, as root): check and compile the program `xapp`
# (source/, a Cargo workspace) and put it into the extension. Rust comes
# from the same Arch snapshot; --locked: exactly the crates of Cargo.lock.
# A format, lint or unit test failure stops the build.
#
# Then the desktop portal with the kind of application "xapp" (portal/,
# docs/xapp.md): xdg-desktop-portal and xdg-document-portal of the same
# version as the snapshot's, with portal/xapp-app-info.patch, in place of
# the system's (/usr/lib, laid over the system by the extension).
#
# Usage: build.sh <extension tree> <Arch snapshot>
set -euo pipefail
root=${1:?tree missing}
# This container, all of it, at the snapshot (-uu: older packages too), so
# that the portal is compiled against the libraries the system has.
pacman --config /tmp/pacman.conf --cachedir /cache --noconfirm -Syuu >/dev/null
pacman --config /tmp/pacman.conf --cachedir /cache --noconfirm --needed -S rust gcc >/dev/null
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

# The portal: same version and same features as the snapshot's package.
# shellcheck disable=SC1091
source /src/extensions/xapp/portal/source
info=$(pacman --config /tmp/pacman.conf -Si xdg-desktop-portal)
have=$(sed -n 's/^Version *: *//p' <<< "$info")
have=${have%-*}
have=${have#*:}
if [[ $have != "$version" ]]; then
    echo "xapp: the snapshot has xdg-desktop-portal $have, the patch is for $version (portal/)" >&2
    exit 1
fi
depends=$(sed -n '/^Depends On/,/^[A-Z][a-z]* [A-Z]/p' <<< "$info" | sed '$d; s/^Depends On *: *//')
feature() { # <package or library pattern>: "enabled" if the snapshot's portal uses it
    if tr ' ' '\n' <<< "$depends" | grep -Eq -- "^($1)([<>=]|$)"; then echo enabled; else echo disabled; fi
}
echo "xdg-desktop-portal $version depends on: $depends"
# shellcheck disable=SC2046
pacman --config /tmp/pacman.conf --cachedir /cache --noconfirm --needed -S \
    meson pkgconf patch glib2-devel $(sed 's/[<>=][^ ]*//g' <<< "$depends") >/dev/null
work=/work/xdp
rm -rf "$work"
mkdir -p "$work"
curl -fsSL -o "$work/xdp.tar.xz" \
    "https://github.com/flatpak/xdg-desktop-portal/releases/download/$version/xdg-desktop-portal-$version.tar.xz"
echo "$sha256  $work/xdp.tar.xz" | sha256sum -c --quiet
tar -xf "$work/xdp.tar.xz" -C "$work"
cd "$work/xdg-desktop-portal-$version"
patch -p1 --quiet < /src/extensions/xapp/portal/xapp-app-info.patch
meson setup "$work/build" --prefix=/usr --libexecdir=lib --buildtype=release \
    -Dtests=disabled -Ddocumentation=disabled -Dman-pages=disabled \
    -Dgeoclue="$(feature 'geoclue|libgeoclue-2.so')" -Dgudev="$(feature 'libgudev|libgudev-1.0.so')" \
    -Dsystemd="$(feature 'systemd-libs|libsystemd.so')"
meson compile -C "$work/build" xdg-desktop-portal xdg-document-portal >/dev/null
install -Dm0755 "$work/build/src/xdg-desktop-portal" "$root/usr/lib/xdg-desktop-portal"
install -Dm0755 "$work/build/document-portal/xdg-document-portal" "$root/usr/lib/xdg-document-portal"
cd /
rm -rf "$work"
