#!/bin/bash
# The part of `xapp doctor` that runs as root inside an Arch Linux
# container: which packages of one Arch Linux Archive snapshot have some
# files (a command not found, a library that could not be loaded).
#
#   /work/want.txt     the files, one per line (usr/bin/tar, usr/lib/libz.so.1)
#   /work/owners.txt   "file<TAB>package" lines
#
# Usage: owners.sh <snapshot YYYY/MM/DD>
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
cd /work
rm -f owners.txt
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
r=/tmp/empty
mkdir -p "$r/var/lib/pacman"
p() { pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" "$@"; }
p -Fy >/dev/null
mapfile -t want < <(grep -v '^$' want.txt || true)
: > owners.txt
if [[ ${#want[@]} -gt 0 ]]; then
    # repo NUL package NUL version NUL file
    p -F --machinereadable "${want[@]}" 2>/dev/null |
        awk -F '\0' 'NF >= 4 { print $4 "\t" $2 }' | sort -u > owners.txt || true
fi
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
