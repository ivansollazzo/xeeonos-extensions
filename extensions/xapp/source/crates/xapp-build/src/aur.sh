# AUR packages (https://aur.archlinux.org), for `xapp create --aur` and
# `xapp build` (recipe "aur"): sourced by create-aur.sh and build-root.sh,
# as root inside an Arch Linux container.
#
# A PKGBUILD is code written by anyone: it is built as a normal user, in a
# root of its own made from the Arch snapshot of the recipe (so that it
# links against the same libraries the application gets), inside the
# container, never on the user's system.
#
# Needs: /tmp/pacman.conf (the snapshot's repositories), git.

# aur_fetch <name> <commit|""> <dir>: the package's AUR repository in <dir>,
# at <commit> (empty: the newest); prints the commit.
aur_fetch() {
    local name=$1 commit=$2 dir=$3
    rm -rf "$dir"
    git clone --quiet "https://aur.archlinux.org/$name.git" "$dir"
    if [[ ! -f $dir/PKGBUILD ]]; then
        echo "aur.sh: there is no AUR package \"$name\"" >&2
        return 2
    fi
    if [[ -n $commit ]]; then
        git -C "$dir" -c advice.detachedHead=false checkout --quiet "$commit"
    fi
    git -C "$dir" rev-parse HEAD
}

# aur_needs <dir>: what the package needs to build and to run (.SRCINFO:
# depends and makedepends, without versions), one per line.
aur_needs() {
    sed -nE 's/^\s*(make)?depends(_x86_64)? = //p' "$1/.SRCINFO" |
        sed -E 's/[<>=].*//' | sort -u
}

# aur_missing <dir>: what it needs that the snapshot does not have (other
# AUR packages, most of the time), one per line.
aur_missing() {
    local r=/tmp/aur-empty dep
    mkdir -p "$r/var/lib/pacman"
    pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" -Sy >/dev/null
    while IFS= read -r dep; do
        [[ -n $dep ]] || continue
        pacman --config /tmp/pacman.conf --root "$r" --dbpath "$r/var/lib/pacman" \
            -Sp --print-format '%n' "$dep" >/dev/null 2>&1 || echo "$dep"
    done < <(aur_needs "$1")
}

# aur_build <dir> <out>: build the package of <dir> (makepkg, as the user
# "builder"), its files (*.pkg.tar.*, not the debug ones) into <out>.
aur_build() {
    local dir=$1 out=$2 b=/tmp/aur-root m
    mkdir -p "$out"
    rm -rf "$b"
    mkdir -p "$b/var/lib/pacman" "$b/dev" "$b/proc" "$b/sys" /tmp/aur-cache
    mapfile -t needs < <(aur_needs "$dir")
    echo "Building $(basename "$dir") from the AUR (Arch snapshot packages)..."
    # Like pacstrap: /dev, /proc, /sys first, for the packages' hooks.
    mount --rbind /dev "$b/dev"
    mount -t proc proc "$b/proc" 2>/dev/null || mount --rbind /proc "$b/proc"
    mount --rbind /sys "$b/sys"
    # The snapshot's repositories, with everything a build needs: headers
    # and pkg-config files too (xapp build leaves them out of applications).
    grep -v '^NoExtract' /tmp/pacman.conf > /tmp/aur-pacman.conf
    pacman --config /tmp/aur-pacman.conf --root "$b" --cachedir /tmp/aur-cache \
        -Sy --noconfirm --needed base base-devel git "${needs[@]}" >/dev/null
    cp -L /etc/resolv.conf "$b/etc/resolv.conf"
    chroot "$b" useradd -m builder
    cp -a "$dir" "$b/home/builder/pkg"
    chroot "$b" chown -R builder: /home/builder/pkg
    local status=0
    # --nodeps: what it needs is installed above; --nocheck: no checkdepends;
    # --skippgpcheck: the authors' keys are not in this root (the sources'
    # checksums in the PKGBUILD are still checked).
    chroot "$b" runuser -u builder -- env MAKEFLAGS="-j$(nproc)" \
        PKGDEST=/home/builder/out bash -c \
        'mkdir -p ~/out && cd ~/pkg && makepkg --nodeps --nocheck --skippgpcheck --noconfirm' ||
        status=$?
    if [[ $status -eq 0 ]]; then
        find "$b/home/builder/out" -name '*.pkg.tar.*' ! -name '*.sig' \
            ! -name '*-debug-*' -exec cp -t "$out" {} +
    fi
    for m in "$b/sys" "$b/proc" "$b/dev"; do
        if mountpoint -q "$m"; then umount -R "$m" 2>/dev/null || umount -R -l "$m"; fi
    done
    if [[ $status -ne 0 ]]; then
        echo "aur.sh: makepkg failed: the PKGBUILD may need packages newer than the snapshot" >&2
        return "$status"
    fi
    rm -rf "$b"
}

# aur_file <out> <name>: the built package called <name> (a PKGBUILD may
# make several), else the only one.
aur_file() {
    local f n
    local -a all=()
    for f in "$1"/*.pkg.tar.*; do
        [[ -f $f ]] || continue
        all+=("$f")
        n=$(pacman -Qp "$f" 2>/dev/null | cut -d' ' -f1)
        if [[ $n == "$2" ]]; then
            echo "$f"
            return 0
        fi
    done
    if [[ ${#all[@]} -eq 1 ]]; then
        echo "${all[0]}"
        return 0
    fi
    echo "aur.sh: no package \"$2\" among those built" >&2
    return 1
}
