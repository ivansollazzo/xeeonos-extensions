#!/usr/bin/env bash
# Check the built extensions without starting a system: each image mounts,
# says it fits <version> (extension-release), carries its manifest, and has
# the files its `check` file lists (extensions/<name>/check: one path per
# line, relative to the image; `#` comments).
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
    check() {
        if sudo test -e "$mnt/$1" || sudo test -L "$mnt/$1"; then
            echo "ok      $1"
        else
            echo "MISSING $1"
            failed=1
        fi
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
