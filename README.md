# XeeonOS extensions

The optional system extensions of [XeeonOS](https://github.com/ivansollazzo/xeeonos):
`systemd-sysext` images (`.xext`) that lay extra software over the sealed
`/usr`, turned on with `sudo xctl ext enable <name>`.

Every extension is built for one exact version of XeeonOS, from the list of
packages that version publishes, and is published next to it, covered by the
same signed manifest. Extensions included in the system image (flatpak,
printing) live in the main repository.

Work in progress: the extensions (docker, rocm, rocm-sdk,
thinkpad-t14-amd-touchpad) and their build move here from the main
repository.

Branch `experimental`: development, as in the main repository.
