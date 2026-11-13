#!/usr/bin/env bash
# Build the optional system extensions (extensions/<name>/) for one XeeonOS
# version: the files their packages add to that system, made with pacman
# against the system's own package database (so only what the system does
# not already have comes in), from the same Arch snapshot. pacman runs in an
# Arch Linux container (tools/mkext-install.sh); each result is packed as a
# read-only erofs image for systemd-sysext, a .xext file that fits exactly
# that version (SYSEXT_LEVEL=<version>).
#
# Output, in <out dir>:
#   xeeonos_<version>.<name>.xext   one per extension
#   <name>.conf                     its manifest (the list of extensions
#                                   that can be downloaded: xctl reads it)
#
# Usage: tools/mkext.sh <pkgdb.tar> <version> <Arch snapshot> <out dir> [name...]
# Needs: docker, mkfs.erofs (erofs-utils), sudo.
set -euo pipefail
cd "$(dirname "$0")/.."

pkgdb=$(realpath "${1:?usage: mkext.sh <pkgdb.tar> <version> <snapshot> <out dir> [name...]}")
version=${2:?version missing}
snapshot=${3:?snapshot missing}
out=${4:?out dir missing}
shift 4
mkdir -p "$out"
names=("$@")
if (( ${#names[@]} == 0 )); then
    for conf in extensions/*/extension.conf; do
        names+=("$(basename "$(dirname "$conf")")")
    done
fi

work=${MKEXT_WORK:-$PWD/mkext.work}
cache=${MKEXT_CACHE:-$work/cache}
mkdir -p "$work" "$cache"
cp "$pkgdb" "$work/pkgdb.tar"

# One extension at a time (some are several GB): install, pack, clean up.
for name in "${names[@]}"; do
    conf=extensions/$name/extension.conf
    tree=$work/tree/$name
    # (apparmor=unconfined: the runner may lack the AppArmor tools docker
    # needs for its default profile.)
    docker run --rm --security-opt apparmor=unconfined \
        --volume "$PWD:/src:ro" --volume "$work:/work" --volume "$(realpath "$cache"):/cache" \
        "${MKEXT_IMAGE:-archlinux:base}" \
        /bin/bash /src/tools/mkext-install.sh "$snapshot" "$name"
    release=$tree/usr/lib/extension-release.d/extension-release.$name
    sudo mkdir -p "$(dirname "$release")"
    printf 'ID=xeeonos\nSYSEXT_LEVEL=%s\n' "$version" | sudo tee "$release" >/dev/null
    xext=$out/xeeonos_$version.$name.xext
    sudo mkfs.erofs --quiet -zlz4hc "$xext" "$tree"
    sudo chown "$(id -u):$(id -g)" "$xext"
    cp "$conf" "$out/$name.conf"
    ls -lh "$xext"
    sudo rm -rf "$work/tree"
done
