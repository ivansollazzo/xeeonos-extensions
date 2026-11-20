#!/bin/bash
# The part of `xapp create --url` that runs as root inside an Arch Linux
# container: download an application published as an archive (for example
# Firefox from mozilla.org), look at its programs and find, in the files
# database of one Arch Linux Archive snapshot, the packages of the libraries
# they need.
#
#   /work/url.txt        the address the download really came from
#   /work/sha256.txt     its SHA-256
#   /work/top.txt        the one directory everything is in ("" if none)
#   /work/files.txt      the files of the application (paths inside it)
#   /work/programs.txt   its programs (ELF executables with an interpreter)
#   /work/needs.txt      "library package": libraries its files link to
#   /work/loads.txt      "library package": libraries named in its files,
#                        loaded while running (dlopen: sound, video, ...)
#   /work/missing.txt    libraries no package of the snapshot has
#   /work/closure.txt    every package the found ones bring
#   /work/providers.txt  virtual packages pacman would choose for alone (scan.sh)
#   /work/hostonly.txt   packages whose files only the system can use (scan.sh)
#   /work/hints.txt      "permission what": what its programs name that a
#                        permission gives (camera /dev/video, audio
#                        libpulse.so.0), for the wizard's proposals
#   /work/multilib.txt   why it needs the multilib repository (32-bit
#                        programs or packages), if it does
#   /work/gpu32.txt      32-bit graphics drivers to propose (scan.sh)
#   /work/tree/          its menu entries and images (copied)
#
# `xapp create --dir` uses it too, for a directory of the user's (mounted
# read-only on /source): then <url> is /source, nothing is downloaded and
# url.txt and sha256.txt are empty.
#
# Usage: create-url.sh <snapshot YYYY/MM/DD> <url | /source>
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
url=${2:?url missing}
cd /work
rm -rf download x tree url.txt sha256.txt top.txt files.txt programs.txt \
    needs.txt loads.txt missing.txt closure.txt hints.txt multilib.txt gpu32.txt \
    providers.txt hostonly.txt
mkdir -p x tree
. /work/scan.sh

pacman-key --init >/dev/null 2>&1
pacman-key --populate archlinux >/dev/null 2>&1
# readelf and strings, from the container's own (current) Arch.
pacman -Sy --noconfirm --needed binutils >/dev/null

if [[ $url == /source ]]; then
    base=/source
    : > url.txt
    : > sha256.txt
else
    echo "Downloading $url..."
    curl -fsSL --retry 3 -o download -w '%{url_effective}' "$url" > url.txt
    sha256sum download | cut -d' ' -f1 > sha256.txt
    bsdtar -xf download -C x
    rm -f download
    base=x
fi
# Everything in one directory (firefox/...): that directory is the app.
mapfile -t entries < <(ls -A "$base")
top=
if [[ ${#entries[@]} -eq 1 && -d $base/${entries[0]} ]]; then
    top=${entries[0]}
fi
echo "$top" > top.txt
root=$base/$top
(cd "$root" && find . -mindepth 1 ! -type d -printf '%P\n' | sort) > files.txt
# Menu entries and images, for the name, the command and the icon.
(cd "$root" && find . -type f \( -name '*.desktop' -o -name '*.png' -o -name '*.svg' \) \
    -printf '%P\0' | xargs -0r cp --parents -t /work/tree)

# The programs, the libraries they need or name, what they hint at.
scan_files "$root" files.txt
cp /tmp/programs programs.txt
cp /tmp/hints hints.txt
# What the application has itself is not needed from a package.
sed 's|.*/||' files.txt | sort -u > /tmp/own
sort -u /tmp/needed | comm -23 - /tmp/own > /tmp/needed.sorted
sort -u /tmp/named | comm -23 - /tmp/own | comm -23 - /tmp/needed.sorted > /tmp/named.sorted
sort -u /tmp/needed32 | comm -23 - /tmp/own > /tmp/needed32.sorted

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
p -Sy >/dev/null
p -Fy >/dev/null
# library -> package: the 64-bit ones in usr/lib, the 32-bit ones in
# usr/lib32 (multilib).
cat /tmp/needed.sorted /tmp/named.sorted > /tmp/libs
owners_of usr/lib /tmp/libs > /tmp/owners
owners_of usr/lib32 /tmp/needed32.sorted > /tmp/owners32
: > missing.txt
: > needs.txt
while IFS= read -r lib; do
    [[ -n $lib ]] || continue
    pkg=$(owner /tmp/owners "$lib")
    if [[ -n $pkg ]]; then echo "$lib $pkg" >> needs.txt; else echo "$lib" >> missing.txt; fi
done < /tmp/needed.sorted
while IFS= read -r lib; do
    [[ -n $lib ]] || continue
    pkg=$(owner /tmp/owners32 "$lib")
    if [[ -n $pkg ]]; then echo "$lib $pkg" >> needs.txt; else echo "$lib (32-bit)" >> missing.txt; fi
done < /tmp/needed32.sorted
mapfile -t chosen < <(cut -d' ' -f2 needs.txt | sort -u)
# Libraries loaded while running.
loads_of /tmp/owners /tmp/named.sorted "${chosen[@]}" > loads.txt
mapfile -t -O "${#chosen[@]}" chosen < <(cut -d' ' -f2 loads.txt)
if [[ ${#chosen[@]} -gt 0 ]]; then
    p -Sp --noconfirm --print-format '%n' "${chosen[@]}" < /dev/null 2>/dev/null | only_names > closure.txt
    providers_of "${chosen[@]}" > providers.txt
else
    : > closure.txt
    : > providers.txt
fi
# 32-bit programs: multilib, and 32-bit graphics drivers.
: > multilib.txt
if [[ -s /tmp/elf32 ]]; then
    echo "32-bit programs: $(head -n3 /tmp/elf32 | tr '\n' ' ')" >> multilib.txt
fi
mapfile -t m < <(multilib_of "${chosen[@]}")
if [[ ${#m[@]} -gt 0 ]]; then
    echo "32-bit packages: ${m[*]}" >> multilib.txt
fi
gpu32 closure.txt > gpu32.txt
# Packages whose files only the system can use (udev rules, kernel modules).
mapfile -t all < closure.txt
host_only "${all[@]}" > hostonly.txt
rm -rf x
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
