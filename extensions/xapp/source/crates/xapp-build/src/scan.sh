# What the files of an application say about what it needs, for
# `xapp create`: sourced by create-url.sh (an archive, a directory) and
# create-arch.sh (a package of the snapshot or of the AUR), as root inside
# an Arch Linux container. Only what the files say, never the name of the
# application: the wizard proposes, the user decides, the recipe keeps it.
#
# Needs: readelf and strings (binutils), p() (pacman on the snapshot, with
# its files database: p -Fy).

# scan_files <root> <file list>: for the ELF files among them,
#   /tmp/needed, /tmp/needed32   libraries they link to (64-bit, 32-bit)
#   /tmp/named, /tmp/named32     libraries they name (loaded while running)
#   /tmp/programs                programs (ELF executables with an interpreter)
#   /tmp/elf32                   the 32-bit ones
#   /tmp/hints                   "permission what": what a permission gives,
#                                named in them (camera /dev/video, audio libpulse.so)
scan_files() {
    local root=$1 list=$2 f p class lib
    : > /tmp/needed
    : > /tmp/needed32
    : > /tmp/named
    : > /tmp/named32
    : > /tmp/programs
    : > /tmp/elf32
    : > /tmp/hints
    while IFS= read -r f; do
        p=$root/$f
        [[ -f $p && ! -L $p ]] || continue
        [[ $(head -c4 "$p" | od -An -tx1 | tr -d ' \n') == 7f454c46 ]] || continue
        # The fifth byte of an ELF file: 1 for 32-bit, 2 for 64-bit.
        class=$(od -An -tu1 -j4 -N1 "$p" | tr -d ' ')
        if [[ $class == 1 ]]; then
            echo "$f" >> /tmp/elf32
            readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> /tmp/needed32
            strings -a "$p" | grep -oE '^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$' >> /tmp/named32 || true
        else
            readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> /tmp/needed
            strings -a "$p" | grep -oE '^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$' >> /tmp/named || true
        fi
        if readelf -l "$p" 2>/dev/null | grep -q 'INTERP' && [[ -x $p ]]; then
            echo "$f" >> /tmp/programs
        fi
        # What a permission gives, named in the program (even one that
        # brings everything itself: Electron opens the camera and loads the
        # sound library on its own).
        if grep -aqsF '/dev/video' "$p"; then echo "camera /dev/video" >> /tmp/hints; fi
        if grep -aqsF 'org.freedesktop.portal.Camera' "$p"; then
            echo "camera the camera portal" >> /tmp/hints
        fi
        for lib in libpulse.so libpipewire-0.3.so libasound.so; do
            if grep -aqsF "$lib" "$p"; then echo "audio $lib" >> /tmp/hints; fi
        done
    done < "$list"
    sort -u -o /tmp/hints /tmp/hints
}

# owners_of <dir> <file of library names>: "library package" for the ones a
# package of the snapshot has in <dir> (usr/lib or usr/lib32), one query
# for all; in usr/lib never a lib32- package.
owners_of() {
    local dir=$1 libs=$2
    local -a all
    mapfile -t all < <(grep -v '^$' "$libs" | sed "s|^|$dir/|")
    [[ ${#all[@]} -gt 0 ]] || return 0
    p -F --machinereadable "${all[@]}" 2>/dev/null | tr '\0' '\t' |
        awk -F'\t' -v d="$dir/" 'index($4, d) == 1 && (d == "usr/lib32/" || $2 !~ /^lib32-/) {
            n = $4; sub(/.*\//, "", n); print n " " $2 }' || true
}

# owner <owners file> <library>: the package for it, never a variant for
# hardware XeeonOS does not drive (CUDA, ROCm, NVIDIA) when another package
# has the library too.
owner() {
    local heavy='(cuda|rocm|hip-|nvidia|opencl-)'
    awk -v l="$2" '$1 == l {print $2}' "$1" | grep -vE "$heavy" | head -n1 || true
}

# loads_of <owners file> <file of library names> <package>...: "library
# package" for the libraries named that the given packages do not bring
# (the dlopen ones), only packages that install together with them (no
# conflicts, like onnxruntime-cpu and -cuda). Prints the packages chosen
# after them on the last line, "chosen <package>...".
loads_of() {
    local owners=$1 libs=$2 lib pkg
    shift 2
    local -a chosen=("$@")
    while IFS= read -r lib; do
        [[ -n $lib ]] || continue
        pkg=$(owner "$owners" "$lib")
        [[ -n $pkg ]] || continue
        if printf '%s\n' "${chosen[@]}" | grep -qxF "$pkg"; then
            continue
        fi
        if p -Sp --noconfirm --print-format '%n' "${chosen[@]}" "$pkg" >/dev/null 2>&1; then
            chosen+=("$pkg")
            echo "$lib $pkg"
        else
            echo "scan.sh: $lib left out ($pkg does not install with the others)" >&2
        fi
    done < "$libs"
}

# multilib_of <package>...: those of the snapshot's multilib repository
# (32-bit libraries, and applications such as those that need them).
multilib_of() {
    [[ $# -gt 0 ]] || return 0
    p -Sp --noconfirm --print-format '%r %n' "$@" 2>/dev/null |
        awk '$1 == "multilib" {print $2}' || true
}

# gpu32 <closure file>: 32-bit graphics drivers (Intel, AMD, Nouveau), for
# 32-bit programs that draw with OpenGL or Vulkan: XeeonOS has no 32-bit
# drivers to lend them. Only those the snapshot has and the closure does
# not.
gpu32() {
    local closure=$1 d
    if grep -qxE 'lib32-(libglvnd|vulkan-icd-loader|mesa)' "$closure" ||
        grep -qhE '^lib(GL|GLX|EGL|vulkan)\.so' /tmp/needed32 /tmp/named32; then
        for d in lib32-mesa lib32-vulkan-radeon lib32-vulkan-intel lib32-vulkan-nouveau; do
            grep -qxF "$d" "$closure" && continue
            if p -Si "$d" >/dev/null 2>&1; then echo "$d"; fi
        done
    fi
}

# providers_of <package>...: the virtual packages (vulkan-driver, ttf-font,
# java-runtime...) installing them leaves pacman to choose a package for by
# itself, one per line: "virtual candidate..." (its default first). Read
# from the questions pacman asks (and answers alone without a terminal).
providers_of() {
    [[ $# -gt 0 ]] || return 0
    p -Sp --noconfirm --print-format '%n' "$@" < /dev/null 2>&1 | parse_providers
}

# only_names: package names only (pacman may print its questions among them).
only_names() {
    grep -E '^[a-z0-9@_+][a-z0-9@._+-]*$' || true
}

# parse_providers: pacman's questions on stdin, "virtual candidate..." out.
parse_providers() {
    awk '
        /providers available for / {
            if (v != "") print v c
            v = $NF; sub(/:$/, "", v); c = ""; next
        }
        v != "" && /^:: Repository / { next }
        v != "" && /^ *[0-9]+\) / {
            n = split($0, w, /[ \t]+/)
            for (i = 1; i <= n; i++) if (w[i] != "" && w[i] !~ /^[0-9]+\)$/) c = c " " w[i]
            next
        }
        v != "" { print v c; v = "" }
        END { if (v != "") print v c }
    ' | sort -u -k1,1
}

# Where only the system looks: an application's sandbox has no udev, no
# kernel modules, no services of the system, no polkit. (build-root.sh
# leaves these out of every image too.)
HOST_ONLY_RE='^(boot/|usr/lib/(udev|modules|modules-load\.d|modprobe\.d|sysusers\.d|tmpfiles\.d|sysctl\.d|binfmt\.d|systemd/(system|system-[a-z-]+|network|logind\.conf\.d|resolved\.conf\.d|journald\.conf\.d))/|usr/share/(dbus-1/system\.d|dbus-1/system-services|polkit-1)/|etc/(udev|modules-load\.d|modprobe\.d|systemd|sysctl\.d)/)'
# What does not count either way: licenses, documentation.
NEUTRAL_RE='^usr/share/(licenses|doc|man|info)/'

# host_only <package>...: those of them whose files are all for the system
# (and at least one is), "package<TAB>what": udev rules, kernel modules...
host_only() {
    [[ $# -gt 0 ]] || return 0
    p -Fl "$@" 2>/dev/null | awk -v host="$HOST_ONLY_RE" -v neutral="$NEUTRAL_RE" '
        $2 ~ /\/$/ || $2 ~ neutral { seen[$1] = 1; next }
        { seen[$1] = 1
          if ($2 ~ host) { h[$1]++
              w = $2
              if (w ~ /udev/) w = "udev rules"
              else if (w ~ /modules|modprobe/) w = "kernel modules"
              else if (w ~ /polkit/) w = "polkit rules"
              else if (w ~ /dbus-1/) w = "system D-Bus services"
              else w = "system services and settings"
              if (!(($1, w) in got)) { got[$1, w] = 1; what[$1] = what[$1] (what[$1] ? ", " : "") w }
          } else other[$1]++ }
        END { for (k in seen) if (h[k] > 0 && !other[k]) printf "%s\t%s\n", k, what[k] }' | sort
}
