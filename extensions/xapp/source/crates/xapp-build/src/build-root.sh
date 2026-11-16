#!/bin/bash
# The part of `xapp build` that runs as root inside an Arch Linux container
# (docs/xapp.md): install the application's packages from one Arch Linux
# Archive snapshot into /work/root, take out what an application does not
# need, and pack the result as a read-only erofs image.
#
#   /recipe   the recipe directory (read-only)
#   /work     out/ (image.erofs, packages.txt, info, icon.*), root/ (temporary)
#
# Usage: build-root.sh <snapshot YYYY/MM/DD> <package>...
# Environment: XAPP_SCRIPT (a script of the recipe, run with ROOT=/work/root),
#   XAPP_REMOVE (paths to delete, one per line, relative to the root),
#   XAPP_SOURCES (archives of the application's authors, one per line:
#   "url sha256 /opt/<dir>"), XAPP_ICON (path inside the root, starting
#   with /, or in the recipe),
#   XAPP_CHOWN (uid:gid to give the results to).
set -euo pipefail
snapshot=${1:?snapshot missing}
shift
root=/work/root
out=/work/out
rm -rf /work/root /work/out
mkdir -p "$root" "$out"

# The tools of this container (current Arch): pacstrap and mkfs.erofs.
pacman-key --init >/dev/null
pacman-key --populate archlinux >/dev/null
pacman -Syu --noconfirm --needed arch-install-scripts erofs-utils >/dev/null

# The application's packages: from the snapshot only.
cat > /tmp/pacman.conf <<CONF
[options]
Architecture = x86_64
SigLevel = Required DatabaseOptional
ParallelDownloads = 8
DisableSandbox
# Never needed by an application.
NoExtract = usr/share/doc/* usr/share/man/* usr/share/info/* usr/share/gtk-doc/*
NoExtract = usr/include/* usr/lib/pkgconfig/* usr/share/pkgconfig/*

[core]
Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch

[extra]
Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch
CONF
# Like pacstrap: /dev, /proc, /sys in the root so that the packages' hooks
# (library cache, icon and font caches, GSettings schemas) run. Only bind
# mounts, because a rootless container (podman on XeeonOS, on the CI
# runners) may not mount devtmpfs or sysfs, which pacstrap does.
mkdir -p "$root/dev" "$root/proc" "$root/sys" "$root/var/lib/pacman" "$root/var/cache/pacman/pkg"
mount --rbind /dev "$root/dev"
mount -t proc proc "$root/proc" 2>/dev/null || mount --rbind /proc "$root/proc"
mount --rbind /sys "$root/sys"
unmount() {
    local m
    for m in "$root/sys" "$root/proc" "$root/dev"; do
        if mountpoint -q "$m"; then umount -R "$m" 2>/dev/null || umount -R -l "$m"; fi
    done
}
trap unmount EXIT
# The keyring of this container checks the packages (no copy in the root).
pacman --config /tmp/pacman.conf --root "$root" --cachedir "$root/var/cache/pacman/pkg" \
    -Sy --noconfirm bash glibc "$@"

q() { pacman --root "$root" --dbpath "$root/var/lib/pacman" -Q "$@"; }
q > "$out/packages.txt"
{
    echo "glibc=$(q glibc | cut -d' ' -f2)"
    echo "libstdcxx=$(q gcc-libs 2>/dev/null | cut -d' ' -f2)"
    if [[ $# -gt 0 ]]; then
        echo "main=$(q "$1" | cut -d' ' -f2)"
    fi
} > "$out/info"

# The application's own archives (recipe "sources"): downloaded, checked
# against their SHA-256, unpacked into their directory (without the one
# directory everything is in, if there is one).
while read -r src_url src_sum src_to; do
    [[ -n ${src_url:-} ]] || continue
    echo "Downloading $src_url..."
    curl -fsSL --retry 3 -o /tmp/source "$src_url"
    if ! echo "$src_sum  /tmp/source" | sha256sum -c --quiet -; then
        echo "build-root.sh: $src_url is not the file of the recipe (SHA-256)" >&2
        exit 1
    fi
    rm -rf /tmp/unpacked && mkdir -p /tmp/unpacked
    bsdtar -xf /tmp/source -C /tmp/unpacked
    rm -f /tmp/source
    mapfile -t entries < <(ls -A /tmp/unpacked)
    from=/tmp/unpacked
    if [[ ${#entries[@]} -eq 1 && -d /tmp/unpacked/${entries[0]} ]]; then
        from=/tmp/unpacked/${entries[0]}
    fi
    mkdir -p "$root${src_to}"
    cp -a "$from/." "$root${src_to}/"
    rm -rf /tmp/unpacked
done <<< "${XAPP_SOURCES:-}"

# A step of the recipe (e.g. settings files, official binaries).
if [[ -n ${XAPP_SCRIPT:-} ]]; then
    ROOT=$root bash "/recipe/$XAPP_SCRIPT"
fi

# The icon, before anything is removed.
if [[ -n ${XAPP_ICON:-} ]]; then
    if [[ $XAPP_ICON == /* ]]; then src=$root$XAPP_ICON; else src=/recipe/$XAPP_ICON; fi
    cp "$src" "$out/icon.${XAPP_ICON##*.}"
fi

# Out: what only a whole system or a developer needs. The sandbox brings
# its own empty /tmp, /var, /run, /proc, /dev, /sys, and the home.
# Never delete through a mount of this container's /dev, /proc, /sys.
unmount
if grep -q " /work/root/" /proc/self/mounts; then
    echo "build-root.sh: something is still mounted in the root" >&2
    exit 1
fi
cd /work/root
rm -rf ./boot ./home ./root ./srv ./mnt ./media ./tmp ./run ./proc ./sys ./dev ./var ./lost+found
rm -rf ./usr/include ./usr/share/doc ./usr/share/man ./usr/share/info ./usr/share/gtk-doc
rm -rf ./usr/lib/pkgconfig ./usr/share/pkgconfig ./usr/share/libalpm
find ./usr/lib -name '*.a' -type f -delete
while IFS= read -r path; do
    [[ -z $path || $path == *..* ]] && continue
    rm -rf "./${path#/}"
done <<< "${XAPP_REMOVE:-}"
mkdir -p ./xapp
cp "$out/packages.txt" ./xapp/packages.txt
# Every file of the image, for xapp build (it looks for programs that make
# their own sandbox: Chromium, Electron, WebKitGTK, Qt WebEngine).
find . -mindepth 1 ! -type d -printf '%P\n' > "$out/files.txt"
cd /work

# Read-only image, compressed (lz4hc: small and fast to read), with fixed
# times so that the same packages make the same image.
mkfs.erofs --quiet -zlz4hc -T0 "$out/image.erofs" "$root"
rm -rf /work/root
du -sh "$out/image.erofs"
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" "$out"
fi
