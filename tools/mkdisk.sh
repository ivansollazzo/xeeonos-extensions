#!/usr/bin/env bash
# A bootable XeeonOS disk, as the installer writes it, made from the files of
# a published update (tools/fetch-system.sh --boot): the ESP (systemd-boot
# from the system itself and the Unified Kernel Image), then /usr and its
# verity hashes in slot A. The first boot adds slot B and the data partition
# (boot-test.sh grows the disk first), as on a real install.
#
# Usage: tools/mkdisk.sh <system dir> <version> <disk.raw>
# Needs: zstd, sfdisk, mkfs.fat (dosfstools), mtools, sudo (to read
# systemd-boot from the /usr image).
set -euo pipefail
sys=${1:?usage: mkdisk.sh <system dir> <version> <disk.raw>}
version=${2:?version missing}
disk=${3:?disk missing}

usr=$(find "$sys" -name "xeeonos_$version.usr-x86-64.*.raw.zst" -print -quit)
verity=$(find "$sys" -name "xeeonos_$version.usr-x86-64-verity.*.raw.zst" -print -quit)
uki=$sys/xeeonos_$version.efi
[[ -n $usr && -n $verity && -e $uki ]] || { echo "system files missing in $sys" >&2; exit 1; }

# 32 hex digits in the file name -> the partition's UUID.
uuid() {
    local hex
    hex=$(sed -E 's/.*\.([0-9a-f]{32})\.raw\.zst$/\1/' <<< "$1")
    echo "${hex:0:8}-${hex:8:4}-${hex:12:4}-${hex:16:4}-${hex:20:12}"
}

work=$(mktemp -d -p "${WORK:-${TMPDIR:-/tmp}}")
zstd -d --sparse --quiet "$usr" -o "$work/usr.raw"
zstd -d --sparse --quiet "$verity" -o "$work/verity.raw"

# Sizes from mkosi.repart of the main repository, in MiB.
esp_mib=1024 verity_mib=128 usr_mib=8192
(( $(stat -c %s "$work/usr.raw") <= usr_mib * 1048576 )) || { echo "usr image too big" >&2; exit 1; }

# systemd-boot of the system itself (signed copy when there is one).
mnt=$work/mnt
mkdir "$mnt"
sudo mount -t erofs -o loop,ro "$work/usr.raw" "$mnt"
boot=$mnt/lib/systemd/boot/efi/systemd-bootx64.efi
[[ -e $boot.signed ]] && boot=$boot.signed
cp "$boot" "$work/systemd-bootx64.efi"
sudo umount "$mnt"

mkfs.fat -C -F 32 -n ESP "$work/esp.img" $((esp_mib * 1024)) >/dev/null
export MTOOLS_SKIP_CHECK=1
mmd -i "$work/esp.img" ::EFI ::EFI/BOOT ::EFI/systemd ::EFI/Linux ::loader
mcopy -i "$work/esp.img" "$work/systemd-bootx64.efi" ::EFI/BOOT/BOOTX64.EFI
mcopy -i "$work/esp.img" "$work/systemd-bootx64.efi" ::EFI/systemd/systemd-bootx64.efi
mcopy -i "$work/esp.img" "$uki" "::EFI/Linux/xeeonos_$version.efi"

rm -f "$disk"
truncate -s $(( (1 + esp_mib + verity_mib + usr_mib + 1) * 1048576 )) "$disk"
sfdisk --quiet "$disk" <<TABLE
label: gpt
start=1MiB, size=${esp_mib}MiB, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, name="esp"
size=${verity_mib}MiB, type=77FF5F63-E7B6-4633-ACF4-1565B864C0E6, uuid=$(uuid "$verity"), name="xeeonos_$version"
size=${usr_mib}MiB, type=8484680C-9521-48C6-9C11-B0720656F69E, uuid=$(uuid "$usr"), name="xeeonos_$version"
TABLE
put() { # <file> <offset in MiB>
    dd if="$1" of="$disk" bs=1M seek="$2" conv=sparse,notrunc status=none
}
put "$work/esp.img" 1
put "$work/verity.raw" $((1 + esp_mib))
put "$work/usr.raw" $((1 + esp_mib + verity_mib))
rm -rf "$work"
sfdisk --list "$disk"
