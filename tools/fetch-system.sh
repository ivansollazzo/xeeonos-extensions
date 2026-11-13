#!/usr/bin/env bash
# Download, from the XeeonOS update published on ghcr.io for <version>, the
# files an extension build needs (not the whole update):
#   xeeonos_<version>.pkgdb.tar.zst   the package database of the system
#   xeeonos_<version>.build.txt       SNAPSHOT=<Arch snapshot>, VERSION=...
# With --boot, also what tools/mkdisk.sh makes a bootable disk of:
#   xeeonos_<version>.efi             the Unified Kernel Image
#   xeeonos_<version>.usr-x86-64*.raw.zst, ...usr-x86-64-verity*.raw.zst
#
# Usage: tools/fetch-system.sh [--boot] <version> <output dir>
# The repository is public: an anonymous token is enough.
set -euo pipefail
boot=no
if [[ ${1:-} == --boot ]]; then
    boot=yes
    shift
fi
version=${1:?usage: fetch-system.sh [--boot] <version> <output dir>}
out=${2:?output dir missing}
repo=${XEEONOS_REPOSITORY:-ivansollazzo/xeeonos}
mkdir -p "$out"

token=$(curl -sSf "https://ghcr.io/token?scope=repository:$repo:pull" | jq -r .token)
auth=(-H "Authorization: Bearer $token")
manifest=$(curl -sSfL "${auth[@]}" \
    -H "Accept: application/vnd.oci.image.manifest.v1+json" \
    "https://ghcr.io/v2/$repo/manifests/$version")

wanted=("xeeonos_$version.pkgdb.tar.zst" "xeeonos_$version.build.txt")
if [[ $boot == yes ]]; then
    wanted+=("xeeonos_$version.efi")
    while read -r file; do
        wanted+=("$file")
    done < <(jq -r '.layers[].annotations["org.opencontainers.image.title"]' <<< "$manifest" |
        grep -E "^xeeonos_$(sed 's/\./\\./g' <<< "$version")\.usr-x86-64(-verity)?\.[0-9a-f]{32}\.raw\.zst$")
fi

for file in "${wanted[@]}"; do
    digest=$(jq -r --arg f "$file" \
        '.layers[] | select(.annotations["org.opencontainers.image.title"] == $f) | .digest' <<< "$manifest")
    if [[ -z $digest ]]; then
        echo "$file is not in ghcr.io/$repo:$version" >&2
        exit 1
    fi
    curl -sSfL "${auth[@]}" "https://ghcr.io/v2/$repo/blobs/$digest" -o "$out/$file"
    # The digest is the file's own SHA-256 (files are stored as they are).
    if [[ $(sha256sum "$out/$file" | cut -d' ' -f1) != "${digest#sha256:}" ]]; then
        echo "$file: wrong checksum" >&2
        exit 1
    fi
    echo "$out/$file"
done
