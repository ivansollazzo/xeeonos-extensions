#!/usr/bin/env bash
# The extensions an extension needs (Requires= in its extension.conf),
# directly or through others, those it needs first, one per line.
#
# Usage: tools/requires.sh <name>
set -euo pipefail
cd "$(dirname "$0")/.."
declare -a seen=() out=()
visit() {
    local name=$1 dep Requires=""
    [[ " ${seen[*]} " == *" $name "* ]] && return
    seen+=("$name")
    [[ -f extensions/$name/extension.conf ]] || { echo "unknown extension: $name" >&2; exit 1; }
    Requires=$(Requires=""; source "extensions/$name/extension.conf"; echo "$Requires")
    for dep in $Requires; do
        visit "$dep"
        [[ " ${out[*]} " == *" $dep "* || $dep == "${seen[0]}" ]] || out+=("$dep")
    done
}
visit "${1:?usage: requires.sh <name>}"
printf '%s\n' "${out[@]}"
