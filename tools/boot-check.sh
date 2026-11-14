#!/usr/bin/env bash
# Try one extension on the real XeeonOS it is made for, in a virtual machine
# (KVM when the runner has it): a disk made from the published system
# (tools/mkdisk.sh), booted with tools/boot-test.sh of the main repository.
#   boot 1: xctl downloads the extension from a signed source and turns it on
#           (at once, or from the next restart: both are accepted)
#   boot 2: it is merged into /usr, its units and hooks have run, and its
#           own test passes (extensions/<name>/test, run as root, if any)
#
# Usage: tools/boot-check.sh <version> <name> <out dir> <system dir>
#   <out dir>     holds xeeonos_<version>.<name>.xext and <name>.conf
#   <system dir>  tools/fetch-system.sh --boot <version> <system dir>
# Env: XEEONOS_REF, the branch of the main repository to take boot-test.sh
#      from (default testing).
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: boot-check.sh <version> <name> <out dir> <system dir>}
name=${2:?name missing}
out=$(realpath "${3:?out dir missing}")
sys=${4:?system dir missing}
work=$(mktemp -d -p "${WORK:-${RUNNER_TEMP:-/tmp}}")

curl -sSfL "https://raw.githubusercontent.com/ivansollazzo/xeeonos/${XEEONOS_REF:-testing}/tools/boot-test.sh" \
    -o "$work/boot-test.sh"
chmod +x "$work/boot-test.sh"
tools/mkdisk.sh "$sys" "$version" "$work/disk.raw"

# A source like the published one: the two files, a manifest signed with a
# throwaway key, given to xctl as the trusted key for this test only.
http=$work/http
mkdir "$http"
ln "$out/xeeonos_$version.$name.xext" "$http/" 2>/dev/null || cp "$out/xeeonos_$version.$name.xext" "$http/"
cp "$out/$name.conf" "$http/"
# The extensions it needs (Requires=), built next to it: xctl offers to turn
# them on too (--yes answers).
yes=""
for dep in $(tools/requires.sh "$name"); do
    ln "$out/xeeonos_$version.$dep.xext" "$http/" 2>/dev/null || cp "$out/xeeonos_$version.$dep.xext" "$http/"
    cp "$out/$dep.conf" "$http/"
    yes=--yes
done
(cd "$http" && sha256sum ./* | sed 's, \./, ,' > SHA256SUMS)
export GNUPGHOME=$work/gnupg
mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key "XeeonOS extensions CI test key" ed25519 sign 1d
gpg --export > "$work/keyring.gpg"
gpg --batch --yes --local-user "XeeonOS extensions CI test key" --detach-sign \
    --output "$http/SHA256SUMS.gpg" "$http/SHA256SUMS"

# The command was `xctl ext` before it became `xctl extensions`.
xctl='if xctl extensions list --offline >/dev/null 2>&1; then x=extensions; else x=ext; fi'

boot() { # <step> <script> <boot-test options...>
    local n=$1
    printf '%s\n' "$2" > "$work/step-$n.sh"
    shift 2
    echo "::group::$name: boot $n"
    # A disk large enough for the biggest extension (rocm-sdk, about 12 GB,
    # downloaded into the data partition next to two 8 GB /usr slots); the
    # file is sparse, so only what is written takes room on the runner.
    DISK_SIZE=${DISK_SIZE:-64G} TIMEOUT=${TIMEOUT:-1200} "$work/boot-test.sh" --credential "ext-test=$work/step-$n.sh" \
        --run "/bin/bash /run/credentials/@system/ext-test" "$@" \
        "$work/disk.raw" "$work/serial-$n.log"
    echo "::endgroup::"
}

boot 1 "
$xctl
systemctl start network-online.target
xctl \$x enable $name $yes --from http://10.0.2.2:8080 --keyring /run/credentials/@system/xctl.keyring
" --http "$http" --credential "xctl.keyring=$work/keyring.gpg" \
    --expect "Signature: valid" \
    --expect "Extension $name: on"

test_options=()
if [[ -f extensions/$name/test ]]; then
    test_options=(--credential "ext-own-test=extensions/$name/test")
fi
boot 2 "
$xctl
xctl \$x list
test -e /usr/lib/extension-release.d/extension-release.$name && echo MERGED-$name
if [[ -e /run/credentials/@system/ext-own-test ]]; then
    bash -e /run/credentials/@system/ext-own-test 2>&1 | sed 's,^,TEST: ,'
    [[ \${PIPESTATUS[0]} == 0 ]] && echo TEST-PASSED-$name
else
    echo TEST-PASSED-$name
fi
xctl status --check
" "${test_options[@]}" \
    --expect "MERGED-$name" \
    --expect "TEST-PASSED-$name"
rm -rf "$work"
