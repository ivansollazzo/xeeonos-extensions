#!/usr/bin/env bash
# What xctl reads for <version>, ghcr.io/<repository>:<version>, made again
# from every extension published for it (tags <version>.<name>, made by
# tools/publish.sh): their files, without copying them (the same blobs, in
# the same repository), and SHA256SUMS of all of them, signed with the
# XeeonOS update key. Any run can make it again: it always lists all of them.
#
# Usage: tools/index.sh <version>
# Needs: oras (logged in to ghcr.io), jq, gpg with the signing key imported
# (SIGNING_KEY: its fingerprint); GITHUB_REPOSITORY.
set -euo pipefail
version=${1:?usage: index.sh <version>}
key=${SIGNING_KEY:?SIGNING_KEY missing}
repo=ghcr.io/${GITHUB_REPOSITORY,,}
work=$(mktemp -d)
cd "$work"

layers='[]'
names=()
while read -r tag; do
    name=${tag#"$version".}
    [[ $tag == "$version.$name" && $name =~ ^[a-z][a-z0-9-]*$ ]] || continue
    manifest=$(oras manifest fetch "$repo:$tag")
    layers=$(jq -c --argjson m "$manifest" '. + $m.layers' <<< "$layers")
    names+=("$name")
done < <(oras repo tags "$repo")

if (( ${#names[@]} == 0 )); then
    echo "No extension published for $version yet."
    exit 0
fi
# Each file's digest is its own SHA-256 (oras stores files as they are).
jq -r '.[] | "\(.digest | ltrimstr("sha256:"))  \(.annotations["org.opencontainers.image.title"])"' \
    <<< "$layers" | sort -k2 > SHA256SUMS
cat SHA256SUMS
gpg --batch --yes --local-user "$key" --detach-sign --output SHA256SUMS.gpg SHA256SUMS

printf '{}' > config.json
oras blob push "$repo" config.json >/dev/null
for f in SHA256SUMS SHA256SUMS.gpg; do
    digest=sha256:$(sha256sum "$f" | cut -d' ' -f1)
    oras blob push "$repo" "$f" >/dev/null
    layers=$(jq -c --arg d "$digest" --arg f "$f" --argjson s "$(stat -c %s "$f")" \
        '. + [{mediaType: "application/vnd.oci.image.layer.v1.tar", digest: $d, size: $s,
               annotations: {"org.opencontainers.image.title": $f}}]' <<< "$layers")
done
jq -n --argjson layers "$layers" '{
    schemaVersion: 2,
    mediaType: "application/vnd.oci.image.manifest.v1+json",
    artifactType: "application/vnd.xeeonos.extensions.v1",
    config: {
        mediaType: "application/vnd.oci.empty.v1+json",
        digest: "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
        size: 2
    },
    layers: $layers
}' > manifest.json
oras manifest push "$repo:$version" manifest.json
echo "$repo:$version lists: ${names[*]}"
rm -rf "$work"
