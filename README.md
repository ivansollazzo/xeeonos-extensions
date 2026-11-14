# XeeonOS extensions

The optional system extensions of [XeeonOS](https://github.com/ivansollazzo/xeeonos):
`systemd-sysext` images (`.xext`) that lay extra software over the sealed
`/usr`, turned on with `sudo xctl extensions enable <name>`. The extensions included
in the system image (flatpak, printing) live in the main repository.

| Extension | What |
| --- | --- |
| `podman` | Containers without root: each user has their own |
| `distrobox` | Other distributions in a container that feels like the system (brings podman) |
| `docker` | The Docker engine, Compose and Buildx; users of the group `docker` use it without sudo |
| `rocm` | AMD ROCm runtimes (HIP, OpenCL), see [docs/rocm.md](docs/rocm.md) |
| `rocm-sdk` | AMD ROCm SDK: `hipcc`, math libraries, headers (several GB) |
| `nvidia` | NVIDIA's driver (open kernel modules, GTX 16xx / RTX 20xx and newer), see [docs/nvidia.md](docs/nvidia.md) |
| `cuda` | NVIDIA CUDA toolkit (several GB); needs `nvidia` and `dev-tools` |
| `thermald` | Intel's thermal daemon: keeps Intel processors cool before they slow down |
| `tlp` | TLP, power saving for laptops tuned by hand (`/etc/tlp.conf`); replaces the GNOME power modes while on |
| `gaming` | Better performance in games: GameMode, the scx_lavd CPU scheduler, no split-lock slowdown, see [docs/gaming.md](docs/gaming.md) |
| `dev-tools` | GCC, make, autotools, CMake, Meson, Ninja, GDB and the system headers, to build programs on the system itself |
| `thinkpad-t14-amd-touchpad` | Touchpad of the ThinkPad T14/P14s AMD and E495 over SMBus (patched kernel modules), see [docs/thinkpad-t14-amd-touchpad.md](docs/thinkpad-t14-amd-touchpad.md) |

## What can be an extension

Only what needs to be part of the system itself:

- **drivers and kernel modules** (e.g. `thinkpad-t14-amd-touchpad`);
- **system services** that run as root or talk to the kernel (e.g. `docker`,
  `podman`);
- **hardware runtimes** that applications use from the system (e.g. `rocm`);
- **the basic developer tools** (`dev-tools`): the compiler and build tools
  with the system's own headers, for building small things in place.

Not an extension:

- **applications**: Flatpak (GNOME Software) or AppImage;
- **anything else, including whole development environments** (languages,
  libraries of a project): in a container, with `distrobox`.

An extension that bundles an application with a system part (e.g. a virtual
machine manager) ships only the system part here; the application comes
from Flatpak.

## Every XeeonOS build gets its own

An extension fits **exactly one** XeeonOS version (`SYSEXT_LEVEL=<version>`):
it is made from that version's own package list and Arch snapshot.

1. XeeonOS publishes a build (testing, or a stable release)
   with its package list, and tells this repository.
2. The workflow **Build** starts by itself: one job per extension, side by
   side. Each one is built against that version (`tools/mkext.sh`), its image
   is checked (`tools/check.sh`), and it is tried on that very XeeonOS in a
   virtual machine with KVM (`tools/boot-check.sh`: xctl downloads it from a
   signed source and turns it on, then after a restart its own `test` runs).
3. Each extension that is green is **published at once**, without waiting for
   the others, as `ghcr.io/ivansollazzo/xeeonos-extensions:<version>.<name>`.
   The workflow **Indice estensioni** then lists all the ones published for
   that version, signed with the XeeonOS update key, in
   `ghcr.io/ivansollazzo/xeeonos-extensions:<version>`: that is what `xctl`
   reads. An extension that fails is simply not available for that version
   yet; "Re-run failed jobs" builds only that one again. At an update, `xctl`
   waits for the extensions turned on to be published for the new version.

Changes to this repository go through pull requests, as usual: their CI
builds and tries the extensions they change (all of them when `tools/` or the
workflows change) on the newest XeeonOS of the target channel, without
publishing. After the merge, the same extensions are published for that
version. **Run workflow** on Build does it by hand (a version, some or all
extensions).

## Making one

One directory per extension in `extensions/<name>/`:

- `extension.conf`: `Description=`, `Special=`, `Default=no`, `Packages=`
  (Arch packages: only what the system does not already have goes in),
  `Enable=` (units to start with it), `Disable=` (system units masked while
  it is on), `Setup=`, `Cleanup=`, `Boot=` (scripts in
  `files/usr/lib/xeeonos/extensions/<name>/`: once, when turned off, every
  boot; see docs/extensions.md in the XeeonOS repository),
  `Live=no` (kernel modules: on and off only at the next restart),
  `KeepDevelopment=`, `Reinstall=`;
- `files/`: files of its own (only under `/usr`);
- `build.sh` (optional): an extra build step, run in the Arch container;
- `check`: paths that must be in the image (`tools/check.sh`);
- `test` (optional): commands run as root on XeeonOS with the extension on
  (`tools/boot-check.sh`); the first one that fails fails the test.

```
tools/fetch-system.sh <version> sys/             # the system's package list
zstd -d sys/xeeonos_<version>.pkgdb.tar.zst -o pkgdb.tar
tools/mkext.sh pkgdb.tar <version> <snapshot> out [name...]
tools/check.sh <version> out [name...]
tools/fetch-system.sh --boot <version> sys/      # + the system images
tools/boot-check.sh <version> <name> out sys/    # needs qemu, ovmf, mtools
```

## Setup (once)

- Secrets (Settings → Secrets and variables → Actions): `XEEONOS_SIGNING_KEY`
  (the same update signing key as the main repository). The main
  repository's `XEEONOS_BOT_TOKEN` must reach this repository (it sends the
  signal of a new build).
- A ruleset on `testing`, `stable` requiring the check
  "Build and check the extensions".
- The package `xeeonos-extensions` on ghcr.io public, after the first
  publication (xctl downloads without logging in).

Branches = channels, as in the main repository: `testing` (the default
branch) and `stable`.
