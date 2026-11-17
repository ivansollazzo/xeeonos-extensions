//! `xapp run --trace`: what the sandbox refused to an application, in
//! words, so that a program that does not start (or misses something) can
//! be understood without reading its source:
//!
//!   - the session bus: the D-Bus filter (xdg-dbus-proxy) runs with --log,
//!     and every name the application wanted to own, talk to or see and was
//!     refused is noted;
//!   - X11: without the capability, the sandbox offers a fake X server (a
//!     socket nobody answers on), and notes when the application knocks;
//!   - permissions: sound (a fake PipeWire and PulseAudio server, the same
//!     way), the camera and notifications (their calls on the bus).
//!
//! The report (~/.local/state/xapps/trace/<app id>) is what `xapp doctor`
//! reads to offer the missing pieces.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use xapp_core::paths;
use xapp_core::util::{Result, read_kv, write_atomic};
use xapp_permissions::overrides::valid_bus_name;

/// Something the sandbox refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// Owning a name on the session bus.
    Own(String),
    /// Talking to a name on the session bus.
    Talk(String),
    /// Seeing whether a name is on the session bus.
    See(String),
    /// An X server.
    X11,
    /// Something a permission gives (its id: audio, camera, notifications).
    Wants(String),
}

impl Finding {
    /// (kind, value) as in the overrides: own/talk/see/capability.
    pub fn entry(&self) -> (&'static str, &str) {
        match self {
            Finding::Own(n) => ("own", n),
            Finding::Talk(n) => ("talk", n),
            Finding::See(n) => ("see", n),
            Finding::X11 => ("capability", "x11"),
            Finding::Wants(p) => ("wants", p),
        }
    }

    fn from_entry(kind: &str, value: &str) -> Option<Finding> {
        match kind {
            "own" => Some(Finding::Own(value.into())),
            "talk" => Some(Finding::Talk(value.into())),
            "see" => Some(Finding::See(value.into())),
            "capability" if value == "x11" => Some(Finding::X11),
            "wants" if xapp_permissions::info(value).is_some() => {
                Some(Finding::Wants(value.into()))
            }
            _ => None,
        }
    }

    /// In words.
    pub fn describe(&self) -> String {
        match self {
            Finding::Own(n) => format!("wanted to take the name {n} on the session bus"),
            Finding::Talk(n) => format!("wanted to talk to {n} on the session bus"),
            Finding::See(n) => format!("looked for {n} on the session bus"),
            Finding::X11 => "looked for an X server (X11 windows)".into(),
            Finding::Wants(p) => format!(
                "wanted what the permission \"{p}\" gives: {}",
                xapp_permissions::label(p)
            ),
        }
    }
}

/// What xdg-dbus-proxy --log says was refused:
///
///   C6: -> org.freedesktop.DBus call org.freedesktop.DBus.RequestName at ...
///   Filtering message due to arg0 org.mpris.MediaPlayer2.vlc, policy: 0 (required 3)
///   C7: -> org.kde.StatusNotifierWatcher call ...Properties.GetAll at ...
///   *HIDDEN* (ping)
///
/// "required" is the level the name needed: 1 see, 2 talk, 3 own. A call
/// followed by *HIDDEN* went to a name the application may not talk to.
pub fn parse_dbus_log(text: &str) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut add = |f: Finding| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    let mut last_call: Option<(String, String)> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Filtering message due to arg0 ") {
            let Some((name, policy)) = rest.split_once(", policy:") else {
                continue;
            };
            let required = policy
                .rsplit_once("(required ")
                .and_then(|(_, r)| r.trim_end_matches(')').trim().parse::<u8>().ok());
            if valid_bus_name(name) {
                match required {
                    Some(3) => add(Finding::Own(name.into())),
                    Some(2) => add(Finding::Talk(name.into())),
                    Some(1) => add(Finding::See(name.into())),
                    _ => {}
                }
            }
            last_call = None;
        } else if line.starts_with("*HIDDEN*") {
            if let Some((dest, method)) = last_call.take() {
                // What a permission gives: the permission, not the name.
                let wanted = match dest.as_str() {
                    "org.freedesktop.portal.Desktop" => {
                        if method.starts_with("org.freedesktop.portal.Camera.") {
                            Some("camera")
                        } else if method.starts_with("org.freedesktop.portal.Notification.") {
                            Some("notifications")
                        } else {
                            None
                        }
                    }
                    "org.freedesktop.Notifications" => Some("notifications"),
                    _ => None,
                };
                if let Some(p) = wanted {
                    add(Finding::Wants(p.into()));
                } else if valid_bus_name(&dest) && dest != "org.freedesktop.portal.Desktop" {
                    add(Finding::Talk(dest));
                }
            }
        } else if let Some((_, rest)) = line.split_once(": -> ") {
            // "C6: -> <destination> call <method> at <path>" (not the proxy's
            // own "fake" calls)
            let mut words = rest.split_whitespace();
            last_call = match (words.next(), words.next(), words.next()) {
                (Some(dest), Some("call"), method) => {
                    Some((dest.to_string(), method.unwrap_or_default().to_string()))
                }
                _ => None,
            };
        }
    }
    // Talking to a name includes seeing it.
    let talked: Vec<String> = out
        .iter()
        .filter_map(|f| match f {
            Finding::Talk(n) | Finding::Own(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    out.retain(|f| !matches!(f, Finding::See(n) if talked.contains(n)));
    out
}

/// The result of a traced run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub app_id: String,
    /// Exit code of the application.
    pub code: i32,
    /// How long it ran.
    pub seconds: u64,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn to_text(&self) -> String {
        let mut t = format!(
            "app={}\nexit={}\nseconds={}\n",
            self.app_id, self.code, self.seconds
        );
        for f in &self.findings {
            let (k, v) = f.entry();
            t.push_str(&format!("{k}={v}\n"));
        }
        t
    }

    pub fn from_pairs(pairs: Vec<(String, String)>) -> Report {
        let mut r = Report::default();
        for (k, v) in pairs {
            match k.as_str() {
                "app" => r.app_id = v,
                "exit" => r.code = v.parse().unwrap_or(0),
                "seconds" => r.seconds = v.parse().unwrap_or(0),
                _ => {
                    if let Some(f) = Finding::from_entry(&k, &v)
                        && !r.findings.contains(&f)
                    {
                        r.findings.push(f);
                    }
                }
            }
        }
        r
    }
}

fn report_path(app_id: &str) -> PathBuf {
    paths::state().join("trace").join(app_id)
}

pub fn save(r: &Report) -> Result<()> {
    write_atomic(&report_path(&r.app_id), r.to_text().as_bytes())
}

/// The last traced run of an application, if there was one.
pub fn load(app_id: &str) -> Option<Report> {
    let path = report_path(app_id);
    path.exists().then(|| Report::from_pairs(read_kv(&path)))
}

/// A fake server (X11, PipeWire, PulseAudio): a socket that notes who
/// knocks and answers nothing.
pub struct Probe {
    pub socket: PathBuf,
    hits: Arc<AtomicUsize>,
}

impl Probe {
    /// A probe at `instance/<name>`.
    pub fn start(instance: &Path, name: &str) -> Option<Probe> {
        let socket = instance.join(name);
        let listener = UnixListener::bind(&socket).ok()?;
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                counter.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        Some(Probe { socket, hits })
    }

    /// Did the application knock?
    pub fn knocked(&self) -> bool {
        self.hits.load(Ordering::SeqCst) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
C1: -> org.freedesktop.DBus call org.freedesktop.DBus.Hello at /org/freedesktop/DBus
C2: -> org.freedesktop.DBus fake AddMatch for org.test.App
B1: <- org.freedesktop.DBus return from C1
*SKIPPED*
C6: -> org.freedesktop.DBus call org.freedesktop.DBus.RequestName at /org/freedesktop/DBus
Filtering message due to arg0 org.mpris.MediaPlayer2.vlc, policy: 0 (required 3)
*HIDDEN* (ping)
B7: <- (no sender) return from C6
*REWRITTEN*
C7: -> org.kde.kwalletd6 call org.kde.KWallet.open at /modules/kwalletd6
*HIDDEN* (ping)
C8: -> org.freedesktop.DBus call org.freedesktop.DBus.NameHasOwner at /org/freedesktop/DBus
Filtering message due to arg0 org.gnome.SessionManager, policy: 0 (required 1)
C9: -> org.freedesktop.DBus call org.freedesktop.DBus.NameHasOwner at /org/freedesktop/DBus
Filtering message due to arg0 org.kde.kwalletd6, policy: 0 (required 1)
C10: -> :1.42 call org.example.Thing at /
*HIDDEN* (ping)
C11: -> org.freedesktop.portal.Desktop call org.freedesktop.portal.Camera.AccessCamera at /org/freedesktop/portal/desktop
*HIDDEN* (ping)
C12: -> org.freedesktop.Notifications call org.freedesktop.Notifications.Notify at /org/freedesktop/Notifications
*HIDDEN* (ping)
C13: -> org.freedesktop.portal.Desktop call org.freedesktop.portal.Wallpaper.SetWallpaperURI at /org/freedesktop/portal/desktop
*HIDDEN* (ping)
";

    #[test]
    fn reads_the_proxy_log() {
        let f = parse_dbus_log(LOG);
        assert_eq!(
            f,
            [
                Finding::Own("org.mpris.MediaPlayer2.vlc".into()),
                Finding::Talk("org.kde.kwalletd6".into()),
                Finding::See("org.gnome.SessionManager".into()),
                Finding::Wants("camera".into()),
                Finding::Wants("notifications".into()),
            ]
        );
        assert!(parse_dbus_log("").is_empty());
    }

    #[test]
    fn report_round_trip() {
        let r = Report {
            app_id: "org.xeeonos.arch.vlc".into(),
            code: 0,
            seconds: 2,
            findings: vec![
                Finding::X11,
                Finding::Own("org.mpris.MediaPlayer2.vlc".into()),
                Finding::Wants("audio".into()),
            ],
        };
        let pairs = r
            .to_text()
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(Report::from_pairs(pairs), r);
        assert!(Finding::X11.describe().contains("X server"));
    }

    #[test]
    fn probe_notes_a_knock() {
        let dir = std::env::temp_dir().join(format!("xapp-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Probe::start(&dir, "x11-probe").unwrap();
        assert!(!p.knocked());
        let _ = std::os::unix::net::UnixStream::connect(&p.socket).unwrap();
        for _ in 0..100 {
            if p.knocked() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(p.knocked());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
