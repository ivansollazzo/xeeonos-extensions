# Extension virtualization

Virtual machines with KVM: the system part (libvirt, QEMU, UEFI firmware,
software TPM, the DHCP/DNS of virtual networks) as an extension, the
interface as an application.

```
sudo xctl extensions enable virtualization
sudo reboot
flatpak install flathub org.virt_manager.virt-manager
```

At the first boot with the extension, the administrators (group `wheel`)
join the group `libvirt` and can manage virtual machines without sudo. A
user created later joins it with `sudo usermod -aG libvirt <user>`.

- The service (`libvirtd`) starts at the first connection.
- Settings, virtual networks and machine definitions: `/etc/libvirt`
  (kept across updates). Disks: `/var/lib/libvirt/images`.
- Windows 11: UEFI (`edk2-ovmf`) and TPM (`swtpm`) are there.
- The virtual network `default` (NAT) is off until started once:
  `virsh --connect qemu:///system net-start default` (and `net-autostart
  default` to keep it).

Turned off or removed, nobody stays in the group `libvirt`; the virtual
machines and their disks are kept.

For simple use (try a distribution, one Windows machine) GNOME Boxes from
Flatpak needs no extension at all.

Not supported: VirtualBox and VMware (out-of-tree kernel modules, rebuilt
for every kernel).
