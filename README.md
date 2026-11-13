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
| `thinkpad-t14-amd-touchpad` | Touchpad of the ThinkPad T14/P14s AMD and E495 over SMBus (patched kernel modules), see [docs/thinkpad-t14-amd-touchpad.md](docs/thinkpad-t14-amd-touchpad.md) |

## Every XeeonOS build gets its own

An extension fits **exactly one** XeeonOS version (`SYSEXT_LEVEL=<version>`):
it is made from that version's own package list and Arch snapshot.

1. XeeonOS publishes a build (experimental, testing, or a stable release)
   with its package list, and tells this repository.
2. The workflow **Nuova build di XeeonOS** opens a pull request towards the
   branch of that channel, pointing `xeeonos.target` at the new version.
3. Its CI (**Build**) builds every extension against that version and checks
   it (`tools/check.sh`). When it is green, the pull request **merges by
   itself**, on every channel, stable too.
4. The merge publishes the extensions, signed with the XeeonOS update key, as
   `ghcr.io/ivansollazzo/xeeonos-extensions:<version>`. `xctl` downloads them
   from there; at an update, it waits for the extensions turned on to be
   published for the new version.

## Making one

One directory per extension in `extensions/<name>/`:

- `extension.conf`: `Description=`, `Special=`, `Default=no`, `Packages=`
  (Arch packages: only what the system does not already have goes in),
  `Enable=` (units to start with it), `KeepDevelopment=`, `Reinstall=`;
- `files/`: files of its own (only under `/usr`);
- `build.sh` (optional): an extra build step, run in the Arch container;
- `check`: paths that must be in the image (`tools/check.sh`).

```
tools/fetch-system.sh <version> sys/             # the system's package list
zstd -d sys/xeeonos_<version>.pkgdb.tar.zst -o pkgdb.tar
tools/mkext.sh pkgdb.tar <version> <snapshot> out [name...]
tools/check.sh <version> out [name...]
```

## Setup (once)

- Secrets (Settings → Secrets and variables → Actions): `XEEONOS_SIGNING_KEY`
  (the same update signing key as the main repository) and
  `XEEONOS_BOT_TOKEN` (the same token, with access to both repositories).
- Settings → General → "Allow auto-merge".
- A ruleset on `experimental`, `testing`, `stable` requiring the check
  "Build and check the extensions" (auto-merge waits for it).

Branches = channels, as in the main repository: `experimental` now, later
`testing` and `stable`.
