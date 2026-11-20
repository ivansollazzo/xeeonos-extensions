//! The system calls an application may not use, as a seccomp filter: a
//! small program for the kernel (classic BPF) that bubblewrap installs
//! before starting the application (`--seccomp <fd>`). Written by hand, so
//! no seccomp library is needed.
//!
//! Refused (EPERM): loading kernel modules, kexec, mounting, keyrings,
//! ptrace, BPF and perf, namespaces (unshare, setns, clone with
//! CLONE_NEWUSER), changing the clock, swap, reboot, typing into the
//! terminal (TIOCSTI) and the like. clone3 answers "not supported", so the
//! C library falls back to clone, whose flags the filter can see. Programs
//! of another architecture are stopped (x32 always; i386 unless the
//! application has 32-bit programs, manifest `multilib`: then their system
//! calls go through the same rules, with their own numbers, as Flatpak's
//! multiarch does).
//!
//! For an application that makes its own sandbox (manifest
//! `user_namespaces`: Chromium, Electron, WebKitGTK, Qt WebEngine, and the
//! bubblewrap of GTK's image loader glycin) unshare, clone with
//! CLONE_NEWUSER and mounting are allowed. Mounting needs CAP_SYS_ADMIN,
//! which the application has only in the namespaces it makes itself (the
//! sandbox drops every capability): it can change only its own sandbox,
//! never this one. setns and the rest stay refused, and the new namespaces
//! inherit this filter.

/// One instruction of a classic BPF program (struct sock_filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

const LD_W_ABS: u16 = 0x20; // load a 32-bit word of seccomp_data
const JEQ: u16 = 0x15; // jump if equal
const JGE: u16 = 0x35; // jump if greater or equal
const JSET: u16 = 0x45; // jump if any of the bits are set
const RET: u16 = 0x06;

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const AUDIT_ARCH_I386: u32 = 0x4000_0003;
const X32_BIT: u32 = 0x4000_0000;
const RET_ALLOW: u32 = 0x7fff_0000;
const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_ERRNO: u32 = 0x0005_0000;
const EPERM: u32 = 1;
const ENOSYS: u32 = 38;

// Offsets in struct seccomp_data: nr, arch, instruction pointer, args[6].
const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;
const fn off_arg_low(i: u32) -> u32 {
    16 + 8 * i
}

const SYS_IOCTL: u32 = 16;
const SYS_CLONE: u32 = 56;
const SYS_CLONE3: u32 = 435;
const TIOCSTI: u32 = 0x5412;
const TIOCLINUX: u32 = 0x541c;
const CLONE_NEWUSER: u32 = 0x1000_0000;

const SYS_UNSHARE: u32 = 272;

/// Allowed to an application that makes its own sandbox: new namespaces
/// and mounting inside them (what bubblewrap does).
const NESTED: &[u32] = &[
    SYS_UNSHARE,
    155, // pivot_root
    165, // mount
    166, // umount2
    428, // open_tree
    429, // move_mount
    430, // fsopen
    431, // fsconfig
    432, // fsmount
    433, // fspick
    442, // mount_setattr
];

/// x86-64 system call numbers refused with EPERM.
pub const DENIED: &[(u32, &str)] = &[
    (101, "ptrace"),
    (103, "syslog"),
    (134, "uselib"),
    (153, "vhangup"),
    (155, "pivot_root"),
    (159, "adjtimex"),
    (163, "acct"),
    (164, "settimeofday"),
    (165, "mount"),
    (166, "umount2"),
    (167, "swapon"),
    (168, "swapoff"),
    (169, "reboot"),
    (170, "sethostname"),
    (171, "setdomainname"),
    (172, "iopl"),
    (173, "ioperm"),
    (175, "init_module"),
    (176, "delete_module"),
    (179, "quotactl"),
    (180, "nfsservctl"),
    (212, "lookup_dcookie"),
    (227, "clock_settime"),
    (246, "kexec_load"),
    (248, "add_key"),
    (249, "request_key"),
    (250, "keyctl"),
    (SYS_UNSHARE, "unshare"),
    (298, "perf_event_open"),
    (304, "open_by_handle_at"),
    (305, "clock_adjtime"),
    (308, "setns"),
    (313, "finit_module"),
    (320, "kexec_file_load"),
    (321, "bpf"),
    (323, "userfaultfd"),
    (428, "open_tree"),
    (429, "move_mount"),
    (430, "fsopen"),
    (431, "fsconfig"),
    (432, "fsmount"),
    (433, "fspick"),
    (442, "mount_setattr"),
];

/// The same rules for i386 programs (manifest `multilib`), with the i386
/// numbers of the same system calls (and the old ones only i386 has).
mod i386 {
    pub const IOCTL: u32 = 54;
    pub const CLONE: u32 = 120;
    pub const CLONE3: u32 = 435;
    pub const UNSHARE: u32 = 310;
    pub const NESTED: &[u32] = &[
        UNSHARE, 217, // pivot_root
        21,  // mount
        22,  // umount
        52,  // umount2
        428, // open_tree
        429, // move_mount
        430, // fsopen
        431, // fsconfig
        432, // fsmount
        433, // fspick
        442, // mount_setattr
    ];
    pub const DENIED: &[(u32, &str)] = &[
        (26, "ptrace"),
        (103, "syslog"),
        (86, "uselib"),
        (111, "vhangup"),
        (217, "pivot_root"),
        (124, "adjtimex"),
        (51, "acct"),
        (79, "settimeofday"),
        (21, "mount"),
        (22, "umount"),
        (52, "umount2"),
        (87, "swapon"),
        (115, "swapoff"),
        (88, "reboot"),
        (74, "sethostname"),
        (121, "setdomainname"),
        (110, "iopl"),
        (101, "ioperm"),
        (113, "vm86old"),
        (166, "vm86"),
        (128, "init_module"),
        (129, "delete_module"),
        (131, "quotactl"),
        (169, "nfsservctl"),
        (253, "lookup_dcookie"),
        (264, "clock_settime"),
        (283, "kexec_load"),
        (286, "add_key"),
        (287, "request_key"),
        (288, "keyctl"),
        (UNSHARE, "unshare"),
        (336, "perf_event_open"),
        (342, "open_by_handle_at"),
        (343, "clock_adjtime"),
        (346, "setns"),
        (350, "finit_module"),
        (357, "bpf"),
        (374, "userfaultfd"),
        (404, "clock_settime64"),
        (405, "clock_adjtime64"),
        (428, "open_tree"),
        (429, "move_mount"),
        (430, "fsopen"),
        (431, "fsconfig"),
        (432, "fsmount"),
        (433, "fspick"),
        (442, "mount_setattr"),
    ];
}

/// The numbers of one architecture.
struct Table {
    ioctl: u32,
    clone: u32,
    clone3: u32,
    nested: &'static [u32],
    denied: &'static [(u32, &'static str)],
    /// x86-64 only: system call numbers with this bit set are x32.
    x32: bool,
}

const X86_64: Table = Table {
    ioctl: SYS_IOCTL,
    clone: SYS_CLONE,
    clone3: SYS_CLONE3,
    nested: NESTED,
    denied: DENIED,
    x32: true,
};

const I386: Table = Table {
    ioctl: i386::IOCTL,
    clone: i386::CLONE,
    clone3: i386::CLONE3,
    nested: i386::NESTED,
    denied: i386::DENIED,
    x32: false,
};

fn ld(off: u32) -> Insn {
    Insn {
        code: LD_W_ABS,
        jt: 0,
        jf: 0,
        k: off,
    }
}
fn jmp(code: u16, k: u32, jt: u8, jf: u8) -> Insn {
    Insn { code, jt, jf, k }
}
fn ret(k: u32) -> Insn {
    Insn {
        code: RET,
        jt: 0,
        jf: 0,
        k,
    }
}

/// The rules for the system calls of one architecture (the number is
/// loaded first). Ends with allow.
fn rules(t: &Table, nested: bool) -> Vec<Insn> {
    let mut p = vec![ld(OFF_NR)];
    if t.x32 {
        // x32 system calls (numbers with bit 30 set): stopped.
        p.extend([jmp(JGE, X32_BIT, 0, 1), ret(RET_KILL_PROCESS)]);
    }
    p.extend([jmp(JEQ, t.clone3, 0, 1), ret(RET_ERRNO | ENOSYS)]);
    for &(nr, _) in t.denied {
        if nested && t.nested.contains(&nr) {
            continue;
        }
        p.push(jmp(JEQ, nr, 0, 1));
        p.push(ret(RET_ERRNO | EPERM));
    }
    // ioctl: refuse TIOCSTI and TIOCLINUX (request in the second argument).
    p.extend([
        jmp(JEQ, t.ioctl, 0, 5),
        ld(off_arg_low(1)),
        jmp(JEQ, TIOCSTI, 2, 0),
        jmp(JEQ, TIOCLINUX, 1, 0),
        ret(RET_ALLOW),
        ret(RET_ERRNO | EPERM),
    ]);
    // clone: refuse a new user namespace (flags in the first argument).
    if !nested {
        p.extend([
            jmp(JEQ, t.clone, 0, 3),
            ld(off_arg_low(0)),
            jmp(JSET, CLONE_NEWUSER, 0, 1),
            ret(RET_ERRNO | EPERM),
        ]);
    }
    p.push(ret(RET_ALLOW));
    p
}

/// The filter program. `nested`: the application may make its own
/// sandbox (user namespaces). `multilib`: it has i386 programs too.
pub fn program(nested: bool, multilib: bool) -> Vec<Insn> {
    let x86 = rules(&X86_64, nested);
    let mut p = vec![ld(OFF_ARCH)];
    if multilib {
        let i386 = rules(&I386, nested);
        // 0 ld arch, 1 x86-64?, 2 i386?, 3 kill, 4.. x86-64, then i386.
        let to_i386 = u8::try_from(x86.len() + 1).expect("filter too long for a jump");
        p.extend([
            jmp(JEQ, AUDIT_ARCH_X86_64, 2, 0),
            jmp(JEQ, AUDIT_ARCH_I386, to_i386, 0),
            ret(RET_KILL_PROCESS),
        ]);
        p.extend(x86);
        p.extend(i386);
    } else {
        // Only x86-64 programs.
        p.extend([jmp(JEQ, AUDIT_ARCH_X86_64, 1, 0), ret(RET_KILL_PROCESS)]);
        p.extend(x86);
    }
    p
}

/// The program as the bytes bubblewrap reads (struct sock_filter[]).
pub fn bytes(nested: bool, multilib: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for i in program(nested, multilib) {
        out.extend_from_slice(&i.code.to_le_bytes());
        out.push(i.jt);
        out.push(i.jf);
        out.extend_from_slice(&i.k.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny BPF interpreter, enough to run the filter in the tests.
    fn eval(nr: u32, arch: u32, args: [u64; 6]) -> u32 {
        eval_with(false, nr, arch, args)
    }

    fn eval_with(nested: bool, nr: u32, arch: u32, args: [u64; 6]) -> u32 {
        run(&program(nested, false), nr, arch, args)
    }

    fn eval32(nested: bool, nr: u32, arch: u32, args: [u64; 6]) -> u32 {
        run(&program(nested, true), nr, arch, args)
    }

    fn run(p: &[Insn], nr: u32, arch: u32, args: [u64; 6]) -> u32 {
        let mut data = Vec::new();
        data.extend_from_slice(&nr.to_le_bytes());
        data.extend_from_slice(&arch.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        for a in args {
            data.extend_from_slice(&a.to_le_bytes());
        }
        let (mut pc, mut acc) = (0usize, 0u32);
        loop {
            let i = p[pc];
            pc += 1;
            match i.code {
                LD_W_ABS => {
                    let o = i.k as usize;
                    acc = u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
                }
                RET => return i.k,
                JEQ | JGE | JSET => {
                    let taken = match i.code {
                        JEQ => acc == i.k,
                        JGE => acc >= i.k,
                        _ => acc & i.k != 0,
                    };
                    pc += if taken { i.jt } else { i.jf } as usize;
                }
                _ => panic!("unknown instruction"),
            }
        }
    }

    const A: u32 = AUDIT_ARCH_X86_64;

    #[test]
    fn decisions() {
        assert_eq!(eval(0, A, [0; 6]), RET_ALLOW); // read
        assert_eq!(eval(101, A, [0; 6]), RET_ERRNO | EPERM); // ptrace
        assert_eq!(eval(165, A, [0; 6]), RET_ERRNO | EPERM); // mount
        assert_eq!(eval(435, A, [0; 6]), RET_ERRNO | ENOSYS); // clone3
        assert_eq!(eval(56, A, [0x11, 0, 0, 0, 0, 0]), RET_ALLOW); // clone (fork)
        assert_eq!(
            eval(56, A, [CLONE_NEWUSER as u64, 0, 0, 0, 0, 0]),
            RET_ERRNO | EPERM
        );
        assert_eq!(
            eval(16, A, [1, TIOCSTI as u64, 0, 0, 0, 0]),
            RET_ERRNO | EPERM
        );
        assert_eq!(eval(16, A, [1, 0x5401, 0, 0, 0, 0]), RET_ALLOW); // TCGETS
        assert_eq!(eval(1, 0x4000_0003, [0; 6]), RET_KILL_PROCESS); // i386
        assert_eq!(eval(X32_BIT | 1, A, [0; 6]), RET_KILL_PROCESS); // x32
        for &(nr, name) in DENIED {
            assert_eq!(eval(nr, A, [0; 6]), RET_ERRNO | EPERM, "{name}");
        }
    }

    #[test]
    fn nested_sandbox() {
        let newuser = [CLONE_NEWUSER as u64 | 0x11, 0, 0, 0, 0, 0];
        assert_eq!(eval_with(true, SYS_CLONE, A, newuser), RET_ALLOW);
        assert_eq!(eval_with(true, SYS_UNSHARE, A, [0; 6]), RET_ALLOW);
        assert_eq!(eval_with(false, SYS_UNSHARE, A, [0; 6]), RET_ERRNO | EPERM);
        // Mounting (in its own namespaces) too.
        for &nr in NESTED {
            assert_eq!(eval_with(true, nr, A, [0; 6]), RET_ALLOW, "{nr}");
            assert_eq!(eval_with(false, nr, A, [0; 6]), RET_ERRNO | EPERM, "{nr}");
        }
        // Still refused: setns, ptrace, modules, keyrings, BPF, clone3
        // (ENOSYS), i386.
        for nr in [308, 101, 175, 250, 321] {
            assert_eq!(eval_with(true, nr, A, [0; 6]), RET_ERRNO | EPERM, "{nr}");
        }
        assert_eq!(eval_with(true, SYS_CLONE3, A, [0; 6]), RET_ERRNO | ENOSYS);
        assert_eq!(eval_with(true, 1, 0x4000_0003, [0; 6]), RET_KILL_PROCESS);
        assert_eq!(
            eval_with(true, SYS_IOCTL, A, [1, TIOCSTI as u64, 0, 0, 0, 0]),
            RET_ERRNO | EPERM
        );
    }

    #[test]
    fn multilib() {
        const I: u32 = AUDIT_ARCH_I386;
        for nested in [false, true] {
            // x86-64: the same decisions as without multilib.
            assert_eq!(eval32(nested, 0, A, [0; 6]), RET_ALLOW);
            assert_eq!(eval32(nested, 101, A, [0; 6]), RET_ERRNO | EPERM);
            assert_eq!(eval32(nested, X32_BIT | 1, A, [0; 6]), RET_KILL_PROCESS);
            // i386: allowed, with the same rules in its own numbers.
            assert_eq!(eval32(nested, 3, I, [0; 6]), RET_ALLOW); // read
            assert_eq!(eval32(nested, 26, I, [0; 6]), RET_ERRNO | EPERM); // ptrace
            assert_eq!(eval32(nested, 435, I, [0; 6]), RET_ERRNO | ENOSYS); // clone3
            assert_eq!(
                eval32(nested, 54, I, [1, TIOCSTI as u64, 0, 0, 0, 0]),
                RET_ERRNO | EPERM
            );
            assert_eq!(eval32(nested, 54, I, [1, 0x5401, 0, 0, 0, 0]), RET_ALLOW);
            for &(nr, name) in i386::DENIED {
                let want = if nested && i386::NESTED.contains(&nr) {
                    RET_ALLOW
                } else {
                    RET_ERRNO | EPERM
                };
                assert_eq!(eval32(nested, nr, I, [0; 6]), want, "{name}");
            }
            // Other architectures: still stopped.
            assert_eq!(eval32(nested, 1, 0xc000_00b7, [0; 6]), RET_KILL_PROCESS);
        }
        let newuser = [CLONE_NEWUSER as u64 | 0x11, 0, 0, 0, 0, 0];
        assert_eq!(eval32(false, 120, I, newuser), RET_ERRNO | EPERM);
        assert_eq!(eval32(true, 120, I, newuser), RET_ALLOW);
        assert_eq!(eval32(false, 120, I, [0x11, 0, 0, 0, 0, 0]), RET_ALLOW);
    }

    #[test]
    fn same_rules_on_both() {
        // Every system call refused on x86-64 is refused on i386 too, but
        // kexec_file_load, which i386 does not have.
        for &(_, name) in DENIED {
            if name == "kexec_file_load" {
                continue;
            }
            assert!(i386::DENIED.iter().any(|&(_, n)| n == name), "{name}");
        }
    }

    #[test]
    fn encoding() {
        for nested in [false, true] {
            for multilib in [false, true] {
                let p = program(nested, multilib);
                assert_eq!(bytes(nested, multilib).len(), p.len() * 8);
                assert!(p.len() < 4096);
            }
        }
    }
}
