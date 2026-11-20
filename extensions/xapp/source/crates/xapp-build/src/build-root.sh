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
#   "url sha256 /opt/<dir>"), XAPP_DIRS (directories of the user, mounted
#   read-only: "/src/<n> /opt/<dir>"), XAPP_AUR (AUR packages, built here
#   with aur.sh: "name commit"), XAPP_MULTILIB (not empty: the snapshot's
#   multilib repository too, for 32-bit packages), XAPP_ICON (path inside the root, starting
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
pacman -Syu --noconfirm --needed arch-install-scripts erofs-utils binutils >/dev/null

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
# 32-bit packages: only if the recipe says so ("multilib": true).
if [[ -n ${XAPP_MULTILIB:-} ]]; then
    printf '\n[multilib]\nServer = https://archive.archlinux.org/repos/%s/$repo/os/$arch\n' \
        "$snapshot" >> /tmp/pacman.conf
fi
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
# Virtual packages the recipe names no package for: pacman would choose
# alone (its first, e.g. nvidia-utils for vulkan-driver). Said, so that the
# recipe can name the one wanted (xapp create asks).
. /work/scan.sh
p() { pacman --config /tmp/pacman.conf --root "$root" "$@"; }
p -Sy >/dev/null
providers_of bash glibc "$@" > "$out/providers.txt" || true
while read -r virtual first others; do
    [[ -n ${virtual:-} ]] || continue
    echo "xapp build: WARNING: the recipe names no package for \"$virtual\": pacman takes $first" \
        "(others: $others). Put the one you want in the recipe's packages." >&2
done < "$out/providers.txt"
# The keyring of this container checks the packages (no copy in the root).
pacman --config /tmp/pacman.conf --root "$root" --cachedir "$root/var/cache/pacman/pkg" \
    -S --noconfirm bash glibc "$@"

# AUR packages: built from the commit of the recipe (aur.sh), installed
# with their dependencies from the snapshot.
aur_names=()
if [[ -n ${XAPP_AUR:-} ]]; then
    pacman -S --noconfirm --needed git >/dev/null
    . /work/aur.sh
    aur_files=()
    while read -r aur_name aur_commit; do
        [[ -n ${aur_name:-} ]] || continue
        got=$(aur_fetch "$aur_name" "$aur_commit" "/tmp/aur/$aur_name")
        if [[ $got != "$aur_commit" ]]; then
            echo "build-root.sh: $aur_name is not at the commit of the recipe" >&2
            exit 1
        fi
        aur_build "/tmp/aur/$aur_name" "/tmp/aurpkgs/$aur_name"
        aur_files+=("$(aur_file "/tmp/aurpkgs/$aur_name" "$aur_name")")
        aur_names+=("$aur_name")
    done <<< "$XAPP_AUR"
    # A package built here has no signature: only these files, by path.
    sed -i 's/^SigLevel = .*/&\nLocalFileSigLevel = Optional/' /tmp/pacman.conf
    pacman --config /tmp/pacman.conf --root "$root" --cachedir "$root/var/cache/pacman/pkg" \
        -U --noconfirm "${aur_files[@]}"
fi

q() { pacman --root "$root" --dbpath "$root/var/lib/pacman" -Q "$@"; }
q > "$out/packages.txt"
{
    echo "glibc=$(q glibc | cut -d' ' -f2)"
    echo "libstdcxx=$(q gcc-libs 2>/dev/null | cut -d' ' -f2)"
    # The application: its AUR package, if it has one (the snapshot's
    # packages may be only for its icon or its libraries).
    if [[ ${#aur_names[@]} -gt 0 ]]; then
        echo "main=$(q "${aur_names[0]}" | cut -d' ' -f2)"
    elif [[ $# -gt 0 ]]; then
        echo "main=$(q "$1" | cut -d' ' -f2)"
    fi
} > "$out/info"

# copy_in <dir> </opt/name>: the files of <dir> into the root, without the
# one directory everything is in, if there is one.
copy_in() {
    local from=$1
    mapfile -t entries < <(ls -A "$1")
    if [[ ${#entries[@]} -eq 1 && -d $1/${entries[0]} ]]; then
        from=$1/${entries[0]}
    fi
    mkdir -p "$root$2"
    cp -a --no-preserve=ownership "$from/." "$root$2/"
}

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
    copy_in /tmp/unpacked "$src_to"
    rm -rf /tmp/unpacked
done <<< "${XAPP_SOURCES:-}"
# The user's directories (xapp create --dir): copied the same way.
while read -r src_from src_to; do
    [[ -n ${src_from:-} ]] || continue
    echo "Copying the directory of $src_to..."
    copy_in "$src_from" "$src_to"
done <<< "${XAPP_DIRS:-}"

# A step of the recipe (e.g. settings files, official binaries).
if [[ -n ${XAPP_SCRIPT:-} ]]; then
    ROOT=$root bash "/recipe/$XAPP_SCRIPT"
fi

# The icon, before anything is removed.
if [[ -n ${XAPP_ICON:-} ]]; then
    if [[ $XAPP_ICON == /* ]]; then src=$root$XAPP_ICON; else src=/recipe/$XAPP_ICON; fi
    cp "$src" "$out/icon.${XAPP_ICON##*.}"
fi

# Every time in the image is 0 (mkfs.erofs -T0, for builds that give the
# same image). Caches that record the times of the files they describe
# would then be stale at every start (VLC: "stale plugins cache", and it
# scans every plugin again): set the times to 0 now and make those caches
# again, so that they match the image.
# (/dev, /proc, /sys are mounts of this container, and go away below.)
find "$root" \( -path "$root/dev" -o -path "$root/proc" -o -path "$root/sys" \) -prune \
    -o -exec touch -h -d @0 {} +
if [[ -x $root/usr/lib/vlc/vlc-cache-gen ]]; then
    chroot "$root" /usr/lib/vlc/vlc-cache-gen /usr/lib/vlc/plugins
fi
# Python checks its compiled files against the time of the source: compile
# them without that check (the image never changes).
for py in "$root"/usr/bin/python3.[0-9]*; do
    [[ -x $py && ! -L $py ]] || continue
    v=${py##*/}
    if [[ -d $root/usr/lib/$v ]]; then
        chroot "$root" "/usr/bin/$v" -m compileall -q -f -j0 \
            --invalidation-mode unchecked-hash "/usr/lib/$v" >/dev/null || true
    fi
done

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
# What only the system uses (scan.sh HOST_ONLY_RE): udev rules, kernel
# modules, services and settings of the system, polkit, system D-Bus.
# Inside the sandbox nothing reads them (Steam brought 491 system units).
find . -mindepth 1 \( -type f -o -type l \) -printf '%P\n' | { grep -E "$HOST_ONLY_RE" || true; } |
    while IFS= read -r path; do rm -f "./$path"; done
find ./boot ./usr/lib/udev ./usr/lib/modules ./usr/lib/systemd ./usr/share/polkit-1 ./etc/systemd \
    ./etc/udev -depth -type d -empty -delete 2>/dev/null || true
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
# What the archives in it hold (launchers that unpack themselves): their
# files, 32-bit programs, programs that draw only on X11 (scan.sh).
inner_report . "$out/files.txt" > "$out/inner.txt"
cd /work

# Read-only image, compressed (lz4hc: small and fast to read), with fixed
# times so that the same packages make the same image.
mkfs.erofs --quiet -zlz4hc -T0 "$out/image.erofs" "$root"
rm -rf /work/root
du -sh "$out/image.erofs"
if [[ -n ${XAPP_CHOWN:-} ]]; then
    chown -R "$XAPP_CHOWN" "$out"
fi
