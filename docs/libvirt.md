# Libvirt and virt-manager

The optional `libvirt` extension provides the host side of QEMU/KVM for the
virt-manager Flatpak. The Flatpak is the graphical client; it connects to the
system libvirt instance through `/run/libvirt`. It does not need the Flatpak
QEMU extension for this system connection.

Enable the extension and restart so its services and package state are set up:

```
sudo xctl extensions enable libvirt
sudo systemctl reboot
```

Install the client from Flathub if it is not installed already:

```
flatpak install flathub org.virt_manager.virt-manager
```

Open **Virtual Machine Manager** and connect to **QEMU/KVM - system**. The
extension includes libvirt, desktop QEMU, the default NAT networking tools,
UEFI firmware, and a software TPM for guests that need one. Hardware
virtualization (Intel VT-x or AMD-V) must be enabled in firmware for KVM
acceleration; without it, QEMU may fall back to slower software emulation.

Check the host connection from a terminal with:

```
virsh --connect qemu:///system list --all
```

## Access to the system instance

The system instance is managed by root. If virt-manager cannot access its
socket, authorize your account through the desktop's Polkit prompt or add it
to the `libvirt` group:

```
sudo usermod -aG libvirt "$USER"
```

Log out and back in after changing group membership. Access to the read-write
system socket grants broad control of the host and its files, so only add
trusted accounts. See [libvirt socket authentication](https://libvirt.org/auth.html).

The included test checks that the system sockets are active and that libvirt
can query its QEMU, network, and storage APIs. It does not start a guest VM;
actual KVM acceleration depends on the host CPU and firmware settings.
