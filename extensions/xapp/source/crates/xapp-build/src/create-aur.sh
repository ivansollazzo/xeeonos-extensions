#!/bin/bash
# The part of `xapp create --aur` that runs as root inside an Arch Linux
# container, in two steps, with the user reading the PKGBUILD in between:
#
#   fetch   /work/aur/          the package's AUR repository (newest commit)
#           /work/commit.txt    that commit
#           /work/missing.txt   what it needs that the snapshot does not have
#   build   /work/aurpkgs/      the package, built (aur.sh), then looked at
#                               like an Arch package (create-arch.sh)
#
# Usage: create-aur.sh <snapshot YYYY/MM/DD> fetch|build <package>
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
step=${2:?step missing}
pkg=${3:?package missing}
cd /work
. /work/aur.sh

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

[multilib]
Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch
CONF

case $step in
    fetch)
        rm -rf aur aurpkgs commit.txt missing.txt
        pacman -Sy --noconfirm --needed git >/dev/null
        aur_fetch "$pkg" "" /work/aur > commit.txt
        aur_missing /work/aur > missing.txt
        ;;
    build)
        rm -rf aurpkgs
        aur_build /work/aur /work/aurpkgs
        file=$(aur_file /work/aurpkgs "$pkg")
        # From here on, as an Arch package (its own XAPP_CHOWN at the end).
        exec bash /work/create-arch.sh "$snapshot" "$pkg" "$file"
        ;;
    *)
        echo "create-aur.sh: unknown step $step" >&2
        exit 2
        ;;
esac
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
