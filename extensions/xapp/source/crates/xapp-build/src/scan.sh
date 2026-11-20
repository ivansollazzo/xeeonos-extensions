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
#
# Archives among them (launchers that unpack themselves when they start,
# like Steam's bootstrap) are opened and their files looked at the same
# way, as "<archive>!<path>"; the libraries an archive brings itself do
# not count as needed.
scan_files() {
    local root=$1 list=$2 a d n o
    : > /tmp/needed
    : > /tmp/needed32
    : > /tmp/named
    : > /tmp/named32
    : > /tmp/programs
    : > /tmp/elf32
    : > /tmp/hints
    scan_list "$root" "$list" ""
    n=0
    while IFS= read -r a; do
        n=$((n + 1))
        [[ $n -le 20 ]] || break
        d=$(mktemp -d)
        if bsdtar -xf "$root/$a" -C "$d" 2>/dev/null; then
            (cd "$d" && find . -type f -printf '%P\n') > /tmp/inner.list
            sed 's|.*/||' /tmp/inner.list | sort -u > /tmp/inner.own
            for o in needed needed32 named named32; do mv "/tmp/$o" "/tmp/$o.outer"; : > "/tmp/$o"; done
            scan_list "$d" /tmp/inner.list "$a!"
            for o in needed needed32 named named32; do
                sort -u "/tmp/$o" | comm -23 - /tmp/inner.own >> "/tmp/$o.outer"
                mv "/tmp/$o.outer" "/tmp/$o"
            done
        fi
        rm -rf "$d"
    done < <(archives_in "$root" "$list")
    sort -u -o /tmp/hints /tmp/hints
}

# archives_in <root> <file list>: the archives among them worth opening
# (tar, zip; more than 100 KiB; not documentation).
archives_in() {
    local f
    grep -E '\.(tar|tar\.(xz|gz|bz2|zst)|tgz|txz|zip)$' "$2" | grep -vE '^usr/share/(doc|help|licenses)/' |
        while IFS= read -r f; do
            [[ -f $1/$f && ! -L $1/$f && $(stat -c %s "$1/$f") -gt 102400 ]] && echo "$f"
        done || true
}

# scan_list <root> <file list> <prefix>: scan_files without the archives;
# the files are noted as <prefix><path>.
scan_list() {
    local root=$1 list=$2 pre=$3 f p class lib
    while IFS= read -r f; do
        p=$root/$f
        [[ -f $p && ! -L $p ]] || continue
        [[ $(head -c4 "$p" | od -An -tx1 | tr -d ' \n') == 7f454c46 ]] || continue
        # The fifth byte of an ELF file: 1 for 32-bit, 2 for 64-bit.
        class=$(od -An -tu1 -j4 -N1 "$p" | tr -d ' ')
        if [[ $class == 1 ]]; then
            echo "$pre$f" >> /tmp/elf32
            readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> /tmp/needed32
            strings -a "$p" | grep -oE '^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$' >> /tmp/named32 || true
        else
            readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> /tmp/needed
            strings -a "$p" | grep -oE '^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$' >> /tmp/named || true
        fi
        if readelf -l "$p" 2>/dev/null | grep -q 'INTERP' && [[ -x $p ]]; then
            # (Not those inside an archive: they are not started as they are.)
            [[ -n $pre ]] || echo "$f" >> /tmp/programs
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
# itself, one per line: "virtual candidate..." (its default first). pacman
# does not ask while it only prints (-Sp), so this reads the sync databases
# (what each package provides and depends on) against what it would
# install: a dependency no package is named after, with more than one
# package providing it, none of which is wanted by name.
providers_of() {
    [[ $# -gt 0 ]] || return 0
    local db tmp f
    db=$(p -v 2>/dev/null | sed -n 's/^DB Path *: *//p' | head -n1)
    tmp=$(mktemp -d)
    for f in "${db%/}"/sync/*.db; do
        [[ -e $f ]] && bsdtar -xOf "$f" '*/desc'
    done > "$tmp/desc"
    p -Sp --noconfirm --print-format '%n' "$@" < /dev/null 2>/dev/null | only_names > "$tmp/closure"
    printf '%s\n' "$@" | sed -E 's/[<>=].*//' > "$tmp/targets"
    providers_from "$tmp/desc" "$tmp/closure" "$tmp/targets"
    rm -rf "$tmp"
}

# providers_from <desc> <closure> <targets>: the work of providers_of, on
# the sync databases' desc entries, the packages pacman would install and
# the ones asked for.
providers_from() {
    awk '
        function bare(d) { sub(/[<>=:].*/, "", d); return d }
        FILENAME == ARGV[1] {
            if ($0 ~ /^%[A-Z]+%$/) { sec = $0; next }
            if ($0 == "") { sec = ""; next }
            if (sec == "%NAME%") { name = $0; names[name] = 1 }
            else if (sec == "%PROVIDES%") { d = bare($0); prov[d] = prov[d] " " name }
            else if (sec == "%DEPENDS%") { deps[name] = deps[name] " " bare($0) }
            next
        }
        FILENAME == ARGV[2] { inc[$1] = 1; order[++n] = $1; next }
        FILENAME == ARGV[3] { if ($1 != "") target[$1] = 1; next }
        END {
            # Wanted by name: asked for, or a dependency named after a package.
            for (t in target) if (t in names) byname[t] = 1
            for (i = 1; i <= n; i++) {
                k = split(deps[order[i]], w, " ")
                for (j = 1; j <= k; j++) {
                    if (w[j] in names) byname[w[j]] = 1
                    else virt[w[j]] = 1
                }
            }
            for (t in target) if (!(t in names)) virt[t] = 1
            for (v in virt) {
                k = split(prov[v], c, " ")
                if (k < 2) continue
                chosen = ""; others = ""; skip = 0
                for (j = 1; j <= k; j++) {
                    if (c[j] in byname) skip = 1
                    if (c[j] in inc) chosen = chosen " " c[j]
                    else others = others " " c[j]
                }
                if (skip || chosen == "") continue
                print v chosen others
            }
        }
    ' "$1" "$2" "$3" | sort -u -k1,1
}

# only_names: package names only (pacman may print its questions among them).
only_names() {
    grep -E '^[a-z0-9@_+][a-z0-9@._+-]*$' || true
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
    # (The expressions through the environment: awk -v would eat their \.)
    p -Fl "$@" 2>/dev/null | HOST="$HOST_ONLY_RE" NEUTRAL="$NEUTRAL_RE" awk '
        BEGIN { host = ENVIRON["HOST"]; neutral = ENVIRON["NEUTRAL"] }
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

# inner_report <root> <file list>: for xapp build, what the archives in the
# image hold, one line each: "file <archive>!<path>" (for the signs of a
# sandbox of its own), "elf32 <archive>!<path>" (32-bit programs: the
# sandbox must let them run), "x11 <archive>!<path>" (a program that links
# to X11 and not to Wayland: it opens its windows only on X11).
inner_report() {
    local root=$1 list=$2 a d f p needs
    while IFS= read -r a; do
        d=$(mktemp -d)
        if bsdtar -xf "$root/$a" -C "$d" 2>/dev/null; then
            while IFS= read -r f; do
                echo "file $a!$f"
                p=$d/$f
                [[ -f $p && ! -L $p ]] || continue
                [[ $(head -c4 "$p" | od -An -tx1 | tr -d ' \n') == 7f454c46 ]] || continue
                [[ $(od -An -tu1 -j4 -N1 "$p" | tr -d ' ') == 1 ]] && echo "elf32 $a!$f"
                needs=$(readelf -d "$p" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
                if grep -qE '^lib(X11|xcb)\.so' <<< "$needs" && ! grep -q '^libwayland-client\.so' <<< "$needs"; then
                    echo "x11 $a!$f"
                fi
            done < <(cd "$d" && find . -type f -printf '%P\n')
        fi
        rm -rf "$d"
    done < <(archives_in "$root" "$list" | head -n 20)
}
