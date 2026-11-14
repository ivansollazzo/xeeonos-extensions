# Gaming

Optional system extension `gaming` (see [extensions.md](https://github.com/ivansollazzo/xeeonos/blob/testing/docs/extensions.md)):
better performance in games. The games themselves come from Flathub
(Steam, Heroic, Lutris in GNOME Software), and so do MangoHud and
Gamescope (as Flatpak extensions of those apps).

```
sudo xctl extensions enable gaming
```

What it adds:

- **GameMode**: while a game runs, the CPU and the GPU run at full power
  and the game comes first. Steam from Flathub asks for it by itself
  through the desktop portal; for other games, start them with
  `gamemoderun <game>` (in Steam: launch options `gamemoderun %command%`).
  Any user at the computer may use it (no group needed).
- **scx_lavd**: a CPU scheduler made for games and interactive use
  ([sched_ext](https://github.com/sched-ext/scx), part of the kernel):
  smoother frames, less stutter while other things run. It runs as the
  service `xeeonos-scx-lavd.service` only while the extension is on; if it
  stops, the kernel's own scheduler takes over at once.
  `cat /sys/kernel/sched_ext/root/ops` shows it.
- **No split-lock slowdown**: some games (mostly Windows games on Proton)
  use "split locks", which the kernel slows down on purpose on recent
  processors; `kernel.split_lock_mitigate=0` turns that off. Turned off,
  the extension puts the kernel's default back.

Turning it off (`sudo xctl extensions disable gaming`) leaves the system as
it was.

## Test

`extensions/gaming/test`, in a XeeonOS virtual machine: GameMode is there,
scx_lavd is the running scheduler, split locks are not slowed down.
