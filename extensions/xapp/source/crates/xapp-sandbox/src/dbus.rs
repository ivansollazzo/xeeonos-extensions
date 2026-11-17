//! The filter between an application and the session bus (D-Bus):
//! xdg-dbus-proxy, with a policy that denies everything except what is
//! listed here. The application gets the proxy's socket as its session bus.
//!
//! Always allowed: owning its own name (org.example.App, needed by GTK and
//! Qt) and its media player name (org.mpris.MediaPlayer2.app), the parts of the desktop portal that ask the user or only open
//! something (opening links and files, settings like dark style, inhibiting
//! sleep, screen sharing, printing, e-mail). With permissions: notifications,
//! the camera portal. Everything else (other applications, GNOME Shell,
//! secrets, the file manager...) does not exist for the application.
//!
//! The proxy runs in a small sandbox of its own whose root holds the
//! application's /.xapp-info: the desktop portal sees the proxy, not the
//! application, and so knows which application is asking (like Flatpak).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use xapp_core::paths;
use xapp_core::util::have;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// The proxy's rules for an application.
pub fn rules(app_id: &str, notifications: bool, camera: bool) -> Vec<String> {
    // Its media player name too (MPRIS: media keys, the desktop's player
    // controls), by the last part of its id like players name themselves:
    // org.mpris.MediaPlayer2.vlc for org.xeeonos.arch.vlc. VLC takes the
    // name failing to own it for another VLC already running, and quits.
    let short = app_id.rsplit('.').next().unwrap_or(app_id).to_lowercase();
    let mut r = vec![
        format!("--own={app_id}"),
        format!("--own={app_id}.*"),
        format!("--own=org.mpris.MediaPlayer2.{short}"),
        format!("--own=org.mpris.MediaPlayer2.{short}.*"),
        // The files the file chooser gave it (GTK asks the document portal
        // for their paths, drag and drop uses it).
        "--talk=org.freedesktop.portal.Documents".to_string(),
    ];
    let mut portal = vec![
        "org.freedesktop.DBus.Properties.*",
        "org.freedesktop.portal.OpenURI.*",
        "org.freedesktop.portal.FileChooser.*",
        "org.freedesktop.portal.Settings.*",
        "org.freedesktop.portal.Inhibit.*",
        "org.freedesktop.portal.ScreenCast.*",
        "org.freedesktop.portal.Print.*",
        "org.freedesktop.portal.Email.*",
        "org.freedesktop.portal.Session.*",
        "org.freedesktop.portal.Request.*",
    ];
    if notifications {
        portal.push("org.freedesktop.portal.Notification.*");
        r.push("--talk=org.freedesktop.Notifications".into());
    }
    if camera {
        portal.push("org.freedesktop.portal.Camera.*");
    }
    for iface in portal {
        r.push(format!("--call={PORTAL}={iface}@{PORTAL_PATH}"));
        r.push(format!("--call={PORTAL}={iface}@{PORTAL_PATH}/*"));
    }
    r.push(format!(
        "--broadcast={PORTAL}=org.freedesktop.portal.*@{PORTAL_PATH}"
    ));
    r.push(format!(
        "--broadcast={PORTAL}=org.freedesktop.portal.*@{PORTAL_PATH}/*"
    ));
    r
}

/// The session bus address of the user, if there is one.
fn session_bus() -> Option<String> {
    if let Ok(a) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        && !a.is_empty()
    {
        return Some(a);
    }
    let socket = paths::runtime_dir().join("bus");
    socket
        .exists()
        .then(|| format!("unix:path={}", socket.display()))
}

/// A running proxy; dropping it stops it.
pub struct Proxy {
    child: Child,
    pub socket: PathBuf,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The proxy's own sandbox: the system's programs, the runtime directory
/// (the bus, the instance) and the application's /.xapp-info (the file
/// `xapp-info` of the instance).
fn proxy_sandbox(instance: &Path, runtime_dir: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "--die-with-parent",
        "--unshare-pid",
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/bin",
        "/sbin",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib",
        "/lib64",
        "--ro-bind-try",
        "/etc",
        "/etc",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let rt = runtime_dir.to_string_lossy().into_owned();
    a.extend(["--bind".into(), rt.clone(), rt]);
    a.extend([
        "--ro-bind".into(),
        instance.join("xapp-info").to_string_lossy().into_owned(),
        "/.xapp-info".into(),
        "--".into(),
        "xdg-dbus-proxy".into(),
    ]);
    a
}

/// Start the document portal if it is not running yet (it is started on
/// demand): its folder for the application must be there before the
/// sandbox binds it.
pub fn start_document_portal() {
    if session_bus().is_none() || !have("gdbus") {
        return;
    }
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout=5",
            "--dest=org.freedesktop.portal.Documents",
            "--object-path=/org/freedesktop/portal/documents",
            "--method=org.freedesktop.portal.Documents.GetMountPoint",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Start the proxy for an application, its socket in `instance`. `None`:
/// no session bus or no xdg-dbus-proxy (the application then has no bus).
pub fn start(instance: &Path, rules: &[String]) -> Option<Proxy> {
    let address = session_bus()?;
    if !have("xdg-dbus-proxy") || !have("bwrap") {
        return None;
    }
    let socket = instance.join("bus");
    let child = Command::new("bwrap")
        .args(proxy_sandbox(instance, &paths::runtime_dir()))
        .arg(address)
        .arg(&socket)
        .arg("--filter")
        .args(rules)
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let proxy = Proxy { child, socket };
    let start = Instant::now();
    while !proxy.socket.exists() {
        if start.elapsed() > Duration::from_secs(5) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        let r = rules("org.mozilla.firefox", false, false);
        assert!(r.contains(&"--own=org.mozilla.firefox".to_string()));
        assert!(r.contains(&"--own=org.mpris.MediaPlayer2.firefox.*".to_string()));
        let r = rules("org.xeeonos.arch.vlc", false, false);
        assert!(r.contains(&"--own=org.mpris.MediaPlayer2.vlc".to_string()));
        assert!(!r.iter().any(|x| x.contains("Notification")));
        assert!(!r.iter().any(|x| x.contains("Camera")));
        assert!(!r.iter().any(|x| x.starts_with("--talk=org.gnome")));
        let r = rules("org.mozilla.firefox", true, true);
        assert!(r.contains(&"--talk=org.freedesktop.Notifications".to_string()));
        assert!(r.iter().any(|x| x.contains("portal.Camera")));
        assert!(r.iter().any(|x| x.contains("portal.FileChooser")));
        assert!(r.contains(&"--talk=org.freedesktop.portal.Documents".to_string()));
    }

    #[test]
    fn proxy_has_the_info_file() {
        let a = proxy_sandbox(Path::new("/r/xapp/run/a-1"), Path::new("/r"));
        assert!(a.windows(3).any(|w| w[0] == "--ro-bind"
            && w[1] == "/r/xapp/run/a-1/xapp-info"
            && w[2] == "/.xapp-info"));
        assert!(
            a.windows(3)
                .any(|w| w[0] == "--bind" && w[1] == "/r" && w[2] == "/r")
        );
        assert_eq!(a.last().unwrap(), "xdg-dbus-proxy");
    }
}
