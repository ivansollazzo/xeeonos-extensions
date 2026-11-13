#!/usr/bin/env bash
# Download, from the XeeonOS update published on ghcr.io for <version>, the
# two files an extension build needs (not the whole update):
#   xeeonos_<version>.pkgdb.tar.zst   the package database of the system
#   xeeonos_<version>.build.txt       SNAPSHOT=<Arch snapshot>, VERSION=...
#
# Usage: tools/fetch-system.sh <version> <output dir>
# The repository is public: an anonymous token is enough.
set -euo pipefail
version=${1:?usage: fetch-system.sh <version> <output dir>}
out=${2:?output dir missing}
repo=${XEEONOS_REPOSITORY:-ivansollazzo/xeeonos}
mkdir -p "$out"

token=$(curl -sSf "https://ghcr.io/token?scope=repository:$repo:pull" | jq -r .token)
auth=(-H "Authorization: Bearer $token")
manifest=$(curl -sSfL "${auth[@]}" \
    -H "Accept: application/vnd.oci.image.manifest.v1+json" \
    "https://ghcr.io/v2/$repo/manifests/$version")

for file in "xeeonos_$version.pkgdb.tar.zst" "xeeonos_$version.build.txt"; do
    digest=$(jq -r --arg f "$file" \
        '.layers[] | select(.annotations["org.opencontainers.image.title"] == $f) | .digest' <<< "$manifest")
    if [[ -z $digest ]]; then
        echo "$file is not in ghcr.io/$repo:$version" >&2
        exit 1
    fi
    curl -sSfL "${auth[@]}" "https://ghcr.io/v2/$repo/blobs/$digest" -o "$out/$file"
    echo "$out/$file"
done
