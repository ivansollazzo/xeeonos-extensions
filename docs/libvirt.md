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
UEFI firmware, and a software TPM for guests that require it. It also installs
libvirt's packaged `default` network definition and marks it for autostart.
When the network daemon starts, this provides guests outbound access through
the host without bridging them onto the physical LAN. The TPM setup defaults
are exposed to the `tss` service account. Hardware virtualization (Intel VT-x
or AMD-V) must be enabled in firmware for KVM acceleration; without it, QEMU
may fall back to slower software emulation.

UEFI guests work with the included OVMF firmware. Secure Boot support firmware
is also included, but its key store is not pre-enrolled. To prepare a guest
with Microsoft Secure Boot keys, create a per-VM variable store from the
OVMF template:

```
sudo virt-fw-vars \
  --input /usr/share/edk2/x64/OVMF_VARS.4m.fd \
  --output /var/lib/libvirt/qemu/nvram/NAME_VARS.fd \
  --secure-boot --enroll-microsoft
sudo chown libvirt-qemu:libvirt-qemu /var/lib/libvirt/qemu/nvram/NAME_VARS.fd
```

Replace `NAME` with the VM name, then configure that VM to use the secure OVMF
loader and the variable store above. In virt-manager, choose firmware with
Secure Boot support in the VM overview, then choose the prepared NVRAM store
in its firmware settings. Secure Boot verifies guest boot components; it does
not turn on Secure Boot for the host.

Check the host connection and default network from a terminal with:

```
virsh --connect qemu:///system list --all
virsh --connect qemu:///system net-list --all
```

## Access to the system instance

The system instance is managed by root. The Flatpak is granted access to its
socket, and this setup does not require adding your account to the `libvirt`
group when the connection already works. If access is denied, check the
socket's Polkit authorization first. Adding an account to `libvirt` is an
optional way to grant passwordless access:

```
sudo usermod -aG libvirt "$USER"
```

Access to the read-write system socket grants broad control of the host and
its files, so only add trusted accounts. See
[libvirt socket authentication](https://libvirt.org/auth.html).

Libvirt's service accounts and groups are created from the package's sysusers
configuration. The `kvm` group and udev permissions for `/dev/kvm` come from
the base system's systemd rules, so this extension does not need duplicate
udev rules. Device-specific udev rules for USB or PCI passthrough depend on the
hardware and should be configured only for the devices you intend to pass
through.

The included test verifies that the default network is active and set to
autostart, initializes a software TPM as the `tss` user, and checks that
libvirt can query its QEMU, network, and storage APIs. It does not start a guest
VM; actual KVM acceleration depends on the host CPU and firmware settings.
