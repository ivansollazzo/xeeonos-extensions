#!/usr/bin/env bash
# Check the built extensions without starting a system: each image mounts,
# says it fits <version> (extension-release), carries its manifest, and has
# the files its `check` file lists (extensions/<name>/check: one path per
# line, relative to the image; `#` comments; `a | b` when either one will do,
# for files a new package version may move).
#
# Usage: tools/check.sh <version> <out dir> [name...]
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: check.sh <version> <out dir> [name...]}
out=${2:?out dir missing}
shift 2
names=("$@")
if (( ${#names[@]} == 0 )); then
    for conf in extensions/*/extension.conf; do
        names+=("$(basename "$(dirname "$conf")")")
    done
fi
mnt=$(mktemp -d)
failed=0
for name in "${names[@]}"; do
    xext=$out/xeeonos_$version.$name.xext
    echo "== $name"
    sudo mount -t erofs -o loop,ro "$xext" "$mnt"
    check() { # <path> [| <path>...]
        local alt
        for alt in $1; do
            [[ $alt == '|' ]] && continue
            if sudo test -e "$mnt/$alt" || sudo test -L "$mnt/$alt"; then
                echo "ok      $alt"
                return
            fi
        done
        echo "MISSING $1"
        # What is there instead, to see where it went.
        for alt in $1; do
            [[ $alt == '|' ]] && continue
            sudo test -d "$mnt/$(dirname "$alt")" &&
                sudo ls -A "$mnt/$(dirname "$alt")" | sed "s,^,  in $(dirname "$alt"): ,"
        done
        failed=1
    }
    if sudo grep -qx "SYSEXT_LEVEL=$version" "$mnt/usr/lib/extension-release.d/extension-release.$name"; then
        echo "ok      SYSEXT_LEVEL=$version"
    else
        echo "WRONG   extension-release (SYSEXT_LEVEL is not $version)"
        failed=1
    fi
    check "usr/lib/xeeonos/extensions/$name.conf"
    if [[ -f extensions/$name/check ]]; then
        while read -r path; do
            [[ -z $path || $path == \#* ]] && continue
            check "$path"
        done < "extensions/$name/check"
    fi
    sudo umount "$mnt"
done
rmdir "$mnt"
exit "$failed"
