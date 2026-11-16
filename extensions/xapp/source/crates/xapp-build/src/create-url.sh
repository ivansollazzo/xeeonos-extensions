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
#   /work/tree/          its menu entries and images (copied)
#
# Usage: create-url.sh <snapshot YYYY/MM/DD> <url>
# Environment: XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
url=${2:?url missing}
cd /work
rm -rf download x tree url.txt sha256.txt top.txt files.txt programs.txt \
    needs.txt loads.txt missing.txt closure.txt
mkdir -p x tree

pacman-key --init >/dev/null 2>&1
pacman-key --populate archlinux >/dev/null 2>&1
# readelf and strings, from the container's own (current) Arch.
pacman -Sy --noconfirm --needed binutils >/dev/null

echo "Downloading $url..."
curl -fsSL --retry 3 -o download -w '%{url_effective}' "$url" > url.txt
sha256sum download | cut -d' ' -f1 > sha256.txt
bsdtar -xf download -C x
rm -f download
# Everything in one directory (firefox/...): that directory is the app.
mapfile -t entries < <(ls -A x)
top=
if [[ ${#entries[@]} -eq 1 && -d x/${entries[0]} ]]; then
    top=${entries[0]}
fi
echo "$top" > top.txt
root=x/$top
(cd "$root" && find . -mindepth 1 ! -type d -printf '%P\n' | sort) > files.txt
# Menu entries and images, for the name, the command and the icon.
(cd "$root" && find . -type f \( -name '*.desktop' -o -name '*.png' -o -name '*.svg' \) \
    -printf '%P\0' | xargs -0r cp --parents -t /work/tree)

# The programs and the libraries they need.
: > programs.txt
: > /tmp/needed
: > /tmp/named
while IFS= read -r f; do
    p=$root/$f
    [[ -f $p && ! -L $p ]] || continue
    [[ $(head -c4 "$p" | od -An -tx1 | tr -d ' \n') == 7f454c46 ]] || continue
    readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> /tmp/needed
    if readelf -l "$p" 2>/dev/null | grep -q 'INTERP' && [[ -x $p ]]; then
        echo "$f" >> programs.txt
    fi
    strings -a "$p" | grep -oE '^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$' >> /tmp/named || true
done < files.txt
# What the application has itself is not needed from a package.
sed 's|.*/||' files.txt | sort -u > /tmp/own
sort -u /tmp/needed | comm -23 - /tmp/own > /tmp/needed.sorted
sort -u /tmp/named | comm -23 - /tmp/own | comm -23 - /tmp/needed.sorted > /tmp/named.sorted

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
r=/tmp/empty
mkdir -p "$r/var/lib/pacman"
p() { pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" "$@"; }
p -Sy >/dev/null
p -Fy >/dev/null
# library -> package, one query for all: the 64-bit one in /usr/lib, and
# never a variant for hardware XeeonOS does not drive (CUDA, ROCm, NVIDIA)
# when another package has the library too.
mapfile -t all < <(cat /tmp/needed.sorted /tmp/named.sorted | sed 's|^|usr/lib/|')
: > /tmp/owners
if [[ ${#all[@]} -gt 0 ]]; then
    p -F --machinereadable "${all[@]}" 2>/dev/null | tr '\0' '\t' |
        awk -F'\t' '$2 !~ /^lib32-/ {n=$4; sub(/.*\//, "", n); print n " " $2}' > /tmp/owners || true
fi
heavy='(cuda|rocm|hip-|nvidia|opencl-)'
owner() {
    local all_owners
    all_owners=$(awk -v l="$1" '$1 == l {print $2}' /tmp/owners)
    grep -vE "$heavy" <<< "$all_owners" | head -n1 || true
}
: > missing.txt
: > needs.txt
while IFS= read -r lib; do
    [[ -n $lib ]] || continue
    pkg=$(owner "$lib")
    if [[ -n $pkg ]]; then echo "$lib $pkg" >> needs.txt; else echo "$lib" >> missing.txt; fi
done < /tmp/needed.sorted
mapfile -t chosen < <(cut -d' ' -f2 needs.txt | sort -u)
# Libraries loaded while running: only packages that install together with
# the others (no conflicts, like onnxruntime-cpu and -cuda).
: > loads.txt
while IFS= read -r lib; do
    [[ -n $lib ]] || continue
    pkg=$(owner "$lib")
    [[ -n $pkg ]] || continue
    if printf '%s\n' "${chosen[@]}" | grep -qxF "$pkg"; then
        echo "$lib $pkg" >> loads.txt
        continue
    fi
    if p -Sp --noconfirm --print-format '%n' "${chosen[@]}" "$pkg" >/dev/null 2>&1; then
        chosen+=("$pkg")
        echo "$lib $pkg" >> loads.txt
    else
        echo "create-url.sh: $lib left out ($pkg does not install with the others)" >&2
    fi
done < /tmp/named.sorted
if [[ ${#chosen[@]} -gt 0 ]]; then
    p -Sp --noconfirm --print-format '%n' "${chosen[@]}" > closure.txt
else
    : > closure.txt
fi
rm -rf x
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" /work
fi
