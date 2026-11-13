#!/usr/bin/env bash
# The newest XeeonOS version published on ghcr.io for a channel:
#   experimental   26.11-experimental.<n>
#   testing        26.11-testing.<n>
#   stable         26.11.2 (releases)
# What pull requests and merges here build against (a new XeeonOS build
# brings its own version instead).
#
# Usage: tools/latest-version.sh <channel>
set -euo pipefail
channel=${1:?usage: latest-version.sh <channel>}
repo=${XEEONOS_REPOSITORY:-ivansollazzo/xeeonos}
case $channel in
    experimental | testing) pattern="^[0-9]{2}\.[0-9]{1,2}-$channel\.[0-9]+$" ;;
    stable) pattern='^[0-9]{2}\.[0-9]{1,2}\.[0-9]+$' ;;
    *) echo "unknown channel: $channel" >&2; exit 2 ;;
esac
token=$(curl -sSf "https://ghcr.io/token?scope=repository:$repo:pull" | jq -r .token)
latest=$(curl -sSfL -H "Authorization: Bearer $token" "https://ghcr.io/v2/$repo/tags/list?n=10000" |
    jq -r '.tags[]' | grep -E "$pattern" | sort -V | tail -n 1 || true)
if [[ -z $latest ]]; then
    echo "no XeeonOS version published for $channel" >&2
    exit 1
fi
echo "$latest"
