#!/bin/bash
# The part of `xapp create --arch` that runs as root inside an Arch Linux
# container: what xapp create needs to know about one package of an Arch
# Linux Archive snapshot, without installing it.
#
#   /work/info.txt      pacman -Si of the package
#   /work/closure.txt   every package installing it brings (one per line)
#   /work/files.txt     the files of the package
#   /work/pkgroot/      its menu entries, icons and metadata (extracted)
#
# Usage: create-arch.sh <snapshot YYYY/MM/DD> <package>
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
pkg=${2:?package missing}
rm -rf /work/pkgroot /work/pkg /work/info.txt /work/closure.txt /work/files.txt
mkdir -p /work/pkgroot /work/pkg

pacman-key --init >/dev/null 2>&1
pacman-key --populate archlinux >/dev/null 2>&1
cat > /tmp/pacman.conf <<CONF
[options]
Architecture = x86_64
SigLevel = Required DatabaseOptional
DisableSandbox

[core]
Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch

[extra]
Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch
CONF
# An empty root: nothing counts as installed, so the closure is complete.
r=/tmp/empty
mkdir -p "$r/var/lib/pacman"
p() { pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" "$@"; }
p -Sy >/dev/null
if ! p -Si "$pkg" > /work/info.txt 2>/dev/null; then
    echo "create-arch.sh: no package \"$pkg\" in the Arch snapshot $snapshot" >&2
    exit 2
fi
p -Sp --print-format '%n' "$pkg" > /work/closure.txt
# The package alone (no dependencies), to read its files.
p -Swdd --noconfirm --cachedir /work/pkg "$pkg" >/dev/null
file=$(find /work/pkg -name "$pkg-*.pkg.tar.*" ! -name '*.sig' | head -n1)
bsdtar -tf "$file" | grep -v '/$' | grep -v '^\.' > /work/files.txt
bsdtar -xf "$file" -C /work/pkgroot \
    --include 'usr/share/applications/*' --include 'usr/share/icons/*' \
    --include 'usr/share/pixmaps/*' --include 'usr/share/metainfo/*'
rm -rf /work/pkg
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
