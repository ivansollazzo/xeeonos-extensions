#!/bin/bash
# The part of `xapp create --arch` that runs as root inside an Arch Linux
# container: what xapp create needs to know about one package of an Arch
# Linux Archive snapshot, without installing it.
#
#   /work/info.txt      pacman -Si of the package
#   /work/closure.txt   every package installing it brings (one per line)
#   /work/files.txt     the files of the package
#   /work/pkgroot/      its menu entries, icons and metadata (extracted)
#   /work/icons.txt     for a menu icon the package does not have (a theme
#                       icon, like kcalc's accessories-calculator): the
#                       packages of the snapshot that have it, "package path"
#   /work/loads.txt     "library package": libraries its programs name and
#                       load while running (dlopen: sound, video, ...) that
#                       its dependencies do not bring (optional ones)
#   /work/named.txt     the libraries its programs name (for loads.txt)
#   /work/hints.txt     "permission what": what its programs name that a
#                       permission gives (scan.sh)
#   /work/multilib.txt  why it needs the multilib repository, if it does
#   /work/gpu32.txt     32-bit graphics drivers to propose (scan.sh)
#
# Usage: create-arch.sh <snapshot YYYY/MM/DD> <package> [<package file>]
#   (a package file: built from the AUR by create-aur.sh, not in the
#   snapshot; its dependencies are)
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
pkg=${2:?package missing}
local_file=${3:-}
rm -rf /work/pkgroot /work/pkg /work/info.txt /work/closure.txt /work/files.txt /work/icons.txt \
    /work/loads.txt /work/hints.txt /work/multilib.txt /work/gpu32.txt /work/named.txt
mkdir -p /work/pkgroot /work/pkg
. /work/scan.sh

pacman-key --init >/dev/null 2>&1
pacman-key --populate archlinux >/dev/null 2>&1
# readelf and strings, from the container's own (current) Arch.
pacman -Sy --noconfirm --needed binutils >/dev/null
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
# An empty root: nothing counts as installed, so the closure is complete.
r=/tmp/empty
mkdir -p "$r/var/lib/pacman"
p() { pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" "$@"; }
p -Sy >/dev/null
if [[ -n $local_file ]]; then
    file=$local_file
    pacman -Qip "$file" > /work/info.txt
    # Its dependencies, from the snapshot; then the package itself.
    mapfile -t deps < <(bsdtar -xOf "$file" .PKGINFO | sed -n 's/^depend = //p' | sed -E 's/[<>=].*//')
    {
        if [[ ${#deps[@]} -gt 0 ]]; then
            p -Sp --print-format '%n' "${deps[@]}"
        fi
        echo "$pkg"
    } | sort -u > /work/closure.txt
else
    if ! p -Si "$pkg" > /work/info.txt 2>/dev/null; then
        echo "create-arch.sh: no package \"$pkg\" in the Arch snapshot $snapshot" >&2
        exit 2
    fi
    p -Sp --print-format '%n' "$pkg" > /work/closure.txt
    # The package alone (no dependencies), to read its files.
    p -Swdd --noconfirm --cachedir /work/pkg "$pkg" >/dev/null
    file=$(find /work/pkg -name "$pkg-*.pkg.tar.*" ! -name '*.sig' | head -n1)
fi
bsdtar -tf "$file" | grep -v '/$' | grep -v '^\.' > /work/files.txt
# Only what the package has (bsdtar fails on a pattern that matches nothing).
grep -E '^usr/share/(applications|icons|pixmaps|metainfo)/' /work/files.txt > /work/wanted.txt || true
if [[ -s /work/wanted.txt ]]; then
    bsdtar -xf "$file" -C /work/pkgroot -T /work/wanted.txt
fi

# Its programs: what they name (libraries loaded while running, what a
# permission gives) and whether they are 32-bit.
mkdir -p /tmp/full
bsdtar -xf "$file" -C /tmp/full
rm -rf /work/pkg
p -Fy >/dev/null
scan_files /tmp/full /work/files.txt
cp /tmp/hints /work/hints.txt
sed 's|.*/||' /work/files.txt | sort -u > /tmp/own
sort -u /tmp/named | comm -23 - /tmp/own > /tmp/named.sorted
cp /tmp/named.sorted /work/named.txt
cp /tmp/elves /work/elves.txt
owners_of usr/lib /tmp/named.sorted > /tmp/owners
# The closure's packages that are in the snapshot (not one built from the
# AUR), for pacman.
mapfile -t in_repo < <(if [[ -n $local_file ]]; then grep -vxF "$pkg" /work/closure.txt; else cat /work/closure.txt; fi || true)
loads_of /tmp/owners /tmp/named.sorted "${in_repo[@]}" > /work/loads.txt
rm -rf /tmp/full
# multilib: the package or what it needs is in it, or it has 32-bit programs.
: > /work/multilib.txt
mapfile -t m < <(multilib_of "${in_repo[@]}")
if [[ ${#m[@]} -gt 0 ]]; then
    echo "32-bit packages: ${m[*]}" >> /work/multilib.txt
fi
if [[ -s /tmp/elf32 ]]; then
    echo "32-bit programs: $(head -n3 /tmp/elf32 | tr '\n' ' ')" >> /work/multilib.txt
fi
gpu32 /work/closure.txt > /work/gpu32.txt
# Menu icons named after a theme icon: who has them (the files database).
: > /work/icons.txt
mapfile -t icons < <(cat /work/pkgroot/usr/share/applications/*.desktop 2>/dev/null |
    sed -n 's/^Icon=//p' | grep -v '^/' | sort -u)
missing=()
for i in "${icons[@]}"; do
    grep -qE "/$i\.(png|svg)$" /work/files.txt || missing+=("$i")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    p -Fy >/dev/null
    for i in "${missing[@]}"; do
        p -Fx --machinereadable "^usr/share/(icons/[^/]+/.*|pixmaps)/$i\.(png|svg)$" 2>/dev/null |
            tr '\0' '\t' | awk -F'\t' '{print $2 " " $4}' >> /work/icons.txt || true
    done
fi
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
