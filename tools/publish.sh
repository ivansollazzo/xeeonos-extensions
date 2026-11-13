#!/usr/bin/env bash
# Publish one extension built for <version> as its own tag,
#   ghcr.io/<repository>:<version>.<name>
# (its image and its manifest), on its own: the others need not be ready.
# tools/index.sh then lists it in what xctl reads, <repository>:<version>.
#
# Usage: tools/publish.sh <version> <name> <out dir>
# Needs: oras, logged in to ghcr.io; GITHUB_REPOSITORY.
set -euo pipefail
version=${1:?usage: publish.sh <version> <name> <out dir>}
name=${2:?name missing}
out=${3:?out dir missing}
repo=ghcr.io/${GITHUB_REPOSITORY,,}
cd "$out"
oras push "$repo:$version.$name" --artifact-type application/vnd.xeeonos.extension.v1 \
    "xeeonos_$version.$name.xext" "$name.conf"
echo "Published $repo:$version.$name"
