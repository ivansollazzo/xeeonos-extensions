#!/bin/bash
# Build step of the extension nvidia (run by tools/mkext-install.sh in its
# Arch container): the module index (modules.dep, ...) made again for the
# kernel with NVIDIA's modules in it. The system's index does not know them,
# and the extension's replaces it while it is on.
#
# Usage: build.sh <extension tree> <Arch snapshot>
set -euo pipefail
root=${1:?tree missing}
kver=$(ls "$root/usr/lib/modules" | grep -- '-arch' | head -n1)
[[ -n $kver ]] || { echo "no NVIDIA modules in the extension" >&2; exit 1; }
echo "Kernel $kver"
# The kernel of the system (same snapshot): its modules and index.
pacman --config /tmp/pacman.conf --cachedir /cache --noconfirm --needed -Syuu linux kmod >/dev/null
[[ -d /usr/lib/modules/$kver/kernel ]] || { echo "kernel $kver not in the snapshot" >&2; exit 1; }
cp -a "$root/usr/lib/modules/$kver/." "/usr/lib/modules/$kver/"
depmod "$kver"
cp -a /usr/lib/modules/"$kver"/modules.* "$root/usr/lib/modules/$kver/"
modinfo -k "$kver" -F version nvidia
