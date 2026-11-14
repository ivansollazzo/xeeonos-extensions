# NVIDIA (driver and CUDA)

Two optional system extensions (see [extensions.md](https://github.com/ivansollazzo/xeeonos/blob/testing/docs/extensions.md)),
from the same Arch snapshot as the system:

| Extension | What | Needs |
| --- | --- | --- |
| `nvidia` | NVIDIA's driver: the open kernel modules (`nvidia-open`, built for the kernel of this XeeonOS version), OpenGL, Vulkan, video decoding, `nvidia-smi` | |
| `cuda` | the CUDA toolkit: `nvcc`, the CUDA libraries and their headers, in `/opt/cuda` (several GB) | `nvidia`, `dev-tools` |

```
sudo xctl extensions enable nvidia
systemctl reboot
nvidia-smi
```

`sudo xctl extensions enable cuda` offers to turn `nvidia` and
`dev-tools` on too: the driver runs CUDA programs, and GCC is the host
compiler `nvcc` needs to compile them. Turning off either dependency turns
off `cuda` too, after asking.

- **Cards**: GeForce GTX 16xx, RTX 20xx and newer (Turing onwards), and
  the matching workstation cards. NVIDIA's open modules do not support
  older cards: those stay on Nouveau, the system's own driver.
- **Nouveau** is turned off while the extension is on. Both are kernel
  modules: turning it on or off waits for the next restart.
- **Flatpak applications** (Steam, games, Blender, ...) download the
  matching NVIDIA libraries for themselves (`org.freedesktop.Platform.GL.nvidia-...`)
  at their next update.
- Video memory is kept across suspend and hibernation (NVIDIA's own units).

## Limits

- The CI has no NVIDIA card: it checks that the driver is built for the
  kernel of the version and that Nouveau is blacklisted, and that `nvcc`
  compiles a program using `dev-tools`; the driver itself is tried only
  on real computers.
- Another extension with kernel modules (`thinkpad-t14-amd-touchpad`)
  brings its own module index: the two cannot be on together yet.
