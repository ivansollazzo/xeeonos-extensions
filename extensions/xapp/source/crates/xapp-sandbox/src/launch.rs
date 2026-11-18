//! Starting an application: mount its image, prepare the sandbox, run it in
//! a systemd scope of its own (`app-xapp-<id>-<n>.scope`: resource limits,
//! its output in the journal, and the name the desktop portal uses to know
//! which application is talking), wait for it, clean up.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use xapp_core::paths;
use xapp_core::util::{Result, err, gid, have, uid};
use xapp_format::{Container, Gpu};
use xapp_permissions as perms;

use crate::trace::{self, Finding, Probe, Report};
use crate::{bwrap, dbus, gpu, mount, seccomp};

/// What to start.
pub struct Launch<'a> {
    pub container: &'a Container,
    /// Trusted or approved author: default permissions apply.
    pub known_author: bool,
    /// Arguments from the user (files, links), after the manifest's own.
    pub args: Vec<String>,
    /// Note what the sandbox refuses (`xapp run --trace`): the report is
    /// saved for `xapp doctor` (trace::load).
    pub trace: bool,
}

/// The instance's directory, removed at the end.
struct Instance(PathBuf);

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// "a%20b" -> "a b" (for file:// links).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                16,
            )
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Files given on the command line (a double click on a document) are
/// lent to the application one by one, under /run/xapp/docs: opening a file
/// is the user's consent to show it that file, like the "powerbox" of
/// macOS. Files inside a folder it was given keep their path.
fn lend_files(args: &[String], folders: &[PathBuf]) -> (Vec<String>, Vec<(PathBuf, String)>) {
    let mut out = Vec::new();
    let mut docs = Vec::new();
    for arg in args {
        let (path, uri) = match arg.strip_prefix("file://") {
            Some(rest) => (PathBuf::from(percent_decode(rest)), true),
            None => (PathBuf::from(arg), false),
        };
        let real = if arg.starts_with('-') {
            None
        } else {
            fs::canonicalize(&path).ok()
        };
        match real {
            Some(real) if !folders.iter().any(|f| real.starts_with(f)) => {
                let name = real
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let dest = format!("/run/xapp/docs/{}/{name}", docs.len());
                out.push(if uri {
                    format!("file://{dest}")
                } else {
                    dest.clone()
                });
                docs.push((real, dest));
            }
            Some(real) => out.push(if uri {
                format!("file://{}", real.display())
            } else {
                real.display().to_string()
            }),
            None => out.push(arg.clone()),
        }
    }
    (out, docs)
}

/// Is standard output a socket (the journal, when started from the menu)?
fn stdout_is_socket() -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::metadata("/proc/self/fd/1").is_ok_and(|m| m.file_type().is_socket())
}

/// Is the user's service manager there (for the scope)?
fn user_manager() -> bool {
    have("systemd-run") && paths::runtime_dir().join("systemd/private").exists()
}

/// Run the application and wait for it; returns its exit code.
pub fn run(l: &Launch) -> Result<i32> {
    let c = l.container;
    let m = &c.manifest;
    let given = |id: &str| perms::granted(m, id, l.known_author);

    let image = mount::mount(c)?;
    let instance = Instance(paths::runtime().join("run").join(format!(
        "{}-{}",
        m.app_id,
        std::process::id()
    )));
    fs::create_dir_all(&instance.0)?;
    let data = paths::app_data(&m.app_id);
    for d in [".config", ".cache", ".local/share", ".local/state"] {
        fs::create_dir_all(data.join(d))?;
    }
    let home = paths::home();
    let network = given("network");
    bwrap::write_instance_files(
        &instance.0,
        uid(),
        gid(),
        &home,
        &bwrap::xapp_info(&m.app_id, &std::process::id().to_string(), network),
    )?;

    let mut folders: Vec<PathBuf> = perms::CATALOG
        .iter()
        .filter(|i| i.folder.is_some() && given(i.id))
        .filter_map(|i| perms::folder_path(i.id))
        .filter(|p| p.is_dir())
        .collect();
    // Custom permissions: folders, devices, names on the bus.
    let custom = perms::granted_custom(m, l.known_author);
    let mut ro_folders = Vec::new();
    for (path, read_only) in custom.iter().filter_map(perms::Custom::folder_path) {
        if !path.is_dir() {
            continue;
        }
        if read_only {
            ro_folders.push(path);
        } else {
            folders.push(path);
        }
    }
    let devices: Vec<String> = custom
        .iter()
        .filter_map(|c| match c {
            perms::Custom::Device(d) => Some(d.clone()),
            _ => None,
        })
        .collect();

    let gpu_given = given("gpu");
    let mut gpu_binds = Vec::new();
    let mut env = Vec::new();
    if gpu_given && m.gpu == Gpu::Host {
        match gpu::compatible(&image.dir, &m.glibc)
            .and_then(|_| gpu::setup(&image.dir, &instance.0))
        {
            Ok(s) => {
                gpu_binds = s.binds;
                env = s.env;
            }
            Err(e) => eprintln!(
                "xapp: the system's graphics drivers are not lent to {}: {e}",
                m.app_id
            ),
        }
    }

    let all_folders: Vec<PathBuf> = folders.iter().chain(&ro_folders).cloned().collect();
    let (args, docs) = lend_files(&l.args, &all_folders);
    dbus::start_document_portal();
    let mut extra = perms::overrides::load(&m.app_id);
    for c in &custom {
        match c {
            perms::Custom::Talk(n) => extra.talk.push(n.clone()),
            perms::Custom::Own(n) => extra.own.push(n.clone()),
            _ => {}
        }
    }
    let dbus_log = l.trace.then(|| instance.0.join("dbus.log"));
    let proxy = dbus::start(
        &instance.0,
        &dbus::rules(&m.app_id, given("notifications"), given("camera"), &extra),
        dbus_log.as_deref(),
    );
    if proxy.is_none() {
        eprintln!(
            "xapp: no session bus for {} (no D-Bus session or no xdg-dbus-proxy)",
            m.app_id
        );
    }

    let x11 = perms::has_capability(m, "x11");
    let probe = if l.trace && !x11 {
        Probe::start(&instance.0, "x11-probe")
    } else {
        None
    };
    let audio = given("audio");
    let audio_probe = if l.trace && !audio {
        Probe::start(&instance.0, "audio-probe")
    } else {
        None
    };
    let ctx = bwrap::Context {
        image: image.dir.clone(),
        instance: instance.0.clone(),
        app_id: m.app_id.clone(),
        uid: uid(),
        home: home.clone(),
        data,
        runtime_dir: paths::runtime_dir(),
        network,
        gpu: gpu_given,
        audio,
        camera: given("camera"),
        x11,
        x11_probe: probe.as_ref().map(|p| p.socket.clone()),
        folders,
        ro_folders,
        devices,
        audio_probe: audio_probe.as_ref().map(|p| p.socket.clone()),
        bus: proxy.as_ref().map(|p| p.socket.clone()),
        gpu_binds,
        env,
        docs,
    };
    let bwrap_args = bwrap::args(&ctx);
    // Its own sandbox inside (user namespaces) only in a scope: the scope,
    // which it cannot leave, is what tells the desktop portal who it is
    // once it can change its root.
    let manager = user_manager();
    let nested = m.user_namespaces && manager;
    if m.user_namespaces && !manager {
        eprintln!(
            "xapp: {} cannot make its own sandbox without the user's service manager",
            m.app_id
        );
    }
    let filter = instance.0.join("seccomp.bpf");
    fs::write(&filter, seccomp::bytes(nested, m.multilib))?;

    // systemd-run --user --scope ... [systemd-cat -t xapp-<id>]
    //   sh -c 'exec bwrap --seccomp 9 "$@" 9<"$0"' <filter> <bwrap args> -- <command>
    // (the shell opens the filter as descriptor 9 for bwrap: the standard
    // library opens files so that they are closed for child programs).
    let mut cmd: Vec<String> = Vec::new();
    if manager {
        cmd.extend(
            [
                "systemd-run",
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "-p",
                "TasksMax=8192",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        cmd.push(format!(
            "--unit={}",
            crate::instances::unit_name(&m.app_id, std::process::id())
        ));
        cmd.push("--".into());
    }
    // Started from the menu, the output goes to the journal through a
    // socket: give it the application's name there (`xapp logs`). On a
    // terminal or into a pipe it stays where it is.
    if stdout_is_socket()
        && have("systemd-cat")
        && std::path::Path::new("/run/systemd/journal/stdout").exists()
    {
        cmd.extend([
            "systemd-cat".to_string(),
            format!("--identifier=xapp-{}", m.app_id),
        ]);
    }
    cmd.extend([
        "/bin/sh".into(),
        "-c".into(),
        "exec bwrap --seccomp 9 \"$@\" 9<\"$0\"".into(),
    ]);
    cmd.push(filter.display().to_string());
    cmd.extend(bwrap_args.0);
    cmd.push("--".into());
    cmd.extend(m.exec.iter().cloned());
    cmd.extend(args);

    // XAPP_DEBUG=1: the whole command line, to see what the sandbox gets.
    if std::env::var_os("XAPP_DEBUG").is_some() {
        eprintln!("xapp: {}", cmd.join(" "));
    }
    let started = std::time::Instant::now();
    let status = Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()
        .map_err(|e| err(format!("cannot start {}: {e}", cmd[0])))?;
    let code = status.code().unwrap_or(1);
    drop(proxy);
    if l.trace {
        let mut findings = Vec::new();
        if probe.as_ref().is_some_and(Probe::knocked) {
            findings.push(Finding::X11);
        }
        if audio_probe.as_ref().is_some_and(Probe::knocked) {
            findings.push(Finding::Wants("audio".into()));
        }
        if let Some(log) = &dbus_log {
            findings.extend(trace::parse_dbus_log(
                &fs::read_to_string(log).unwrap_or_default(),
            ));
        }
        trace::save(&Report {
            app_id: m.app_id.clone(),
            code,
            seconds: started.elapsed().as_secs(),
            findings,
        })?;
    }
    drop(image);
    drop(instance);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn files_are_lent() {
        let dir = std::env::temp_dir().join(format!("xapp-lend-{}", std::process::id()));
        fs::create_dir_all(dir.join("given")).unwrap();
        fs::write(dir.join("a b.txt"), "x").unwrap();
        fs::write(dir.join("given/c.txt"), "x").unwrap();
        let a = dir.join("a b.txt").display().to_string();
        let c = dir.join("given/c.txt").display().to_string();
        let (args, docs) = lend_files(
            &[
                a.clone(),
                "https://example.com".into(),
                c.clone(),
                "--flag".into(),
            ],
            &[fs::canonicalize(dir.join("given")).unwrap()],
        );
        assert_eq!(args[0], "/run/xapp/docs/0/a b.txt");
        assert_eq!(args[1], "https://example.com");
        assert_eq!(args[2], fs::canonicalize(&c).unwrap().display().to_string());
        assert_eq!(args[3], "--flag");
        assert_eq!(docs.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
