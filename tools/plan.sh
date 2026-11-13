#!/usr/bin/env bash
# What a CI run builds: the XeeonOS version, its channel, and which
# extensions. Prints, for $GITHUB_OUTPUT:
#   version=<XeeonOS version>  channel=<channel>  names=<JSON list>
#
# Usage: tools/plan.sh <channel> <version or ""> <names or ""> [<diff base> <diff head>]
#   version ""     the newest one published for the channel
#   names ""       with a diff: the extensions it changes (all of them if it
#                  changes anything else, such as tools/ or the workflows);
#                  without: all of them
set -euo pipefail
cd "$(dirname "$0")/.."
channel=${1:?usage: plan.sh <channel> <version> <names> [<base> <head>]}
version=${2:-}
names=${3:-}
base=${4:-}
head=${5:-}

all() { for c in extensions/*/extension.conf; do basename "$(dirname "$c")"; done; }

if [[ -z $version ]]; then
    version=$(tools/latest-version.sh "$channel")
fi
if [[ -z $names ]]; then
    if [[ -n $base && ! $base =~ ^0+$ ]] && git cat-file -e "$base^{commit}" 2>/dev/null; then
        names=$(git diff --name-only "$base" "$head" | while read -r file; do
            case $file in
                extensions/*/*) n=${file#extensions/}; n=${n%%/*}
                                [[ -e extensions/$n/extension.conf ]] && echo "$n" ;;
                docs/* | *.md) ;;
                *) all ;;
            esac
        done | sort -u)
    else
        names=$(all)
    fi
fi
for n in $names; do
    [[ -e extensions/$n/extension.conf ]] || { echo "no extension $n" >&2; exit 1; }
done
echo "version=$version"
echo "channel=$channel"
echo "names=$(printf '%s\n' $names | jq -R . | jq -sc 'map(select(. != ""))')"
