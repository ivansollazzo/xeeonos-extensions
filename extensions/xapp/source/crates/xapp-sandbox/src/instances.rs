//! The applications running now (`xapp ps`, `xapp kill`), like `flatpak ps`
//! and `flatpak kill`.
//!
//! Each start has its directory, <runtime>/run/<app id>-<pid> (launch.rs),
//! where <pid> is the `xapp run` that waits for it; with the user's service
//! manager it also has its scope, app-xapp-<id>-<pid>.scope, which holds
//! every process of the application.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use xapp_core::paths;
use xapp_core::util::{Result, err, output_opt, run as run_program};

/// One running application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub app_id: String,
    /// The `xapp run` that started it.
    pub pid: u32,
    /// Its scope, if it has one (and it is still there).
    pub unit: Option<String>,
    /// How long it has been running.
    pub uptime: Duration,
}

/// "<app id>-<pid>" -> (app id, pid).
fn split(name: &str) -> Option<(&str, u32)> {
    let (id, pid) = name.rsplit_once('-')?;
    let pid = pid.parse().ok()?;
    (!id.is_empty()).then_some((id, pid))
}

/// The scope's unit name: dashes in the id escaped as systemd does.
pub fn unit_name(app_id: &str, pid: u32) -> String {
    format!("app-xapp-{}-{pid}.scope", app_id.replace('-', "\\x2d"))
}

/// A property of a user unit (None: no manager, or no such unit).
fn unit_property(unit: &str, property: &str) -> Option<String> {
    output_opt(
        "systemctl",
        &["--user", "show", "--value", "-p", property, unit],
    )
    .filter(|v| !v.is_empty())
}

/// The applications running now, the oldest first. A directory left by an
/// `xapp run` that is gone (killed) is removed.
pub fn running() -> Vec<Running> {
    let dir = paths::runtime().join("run");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((app_id, pid)) = split(&name) else {
            continue;
        };
        if !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = fs::remove_dir_all(entry.path());
            continue;
        }
        let since = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::now());
        let unit = unit_name(app_id, pid);
        let active = unit_property(&unit, "ActiveState").is_some_and(|s| s == "active");
        out.push(Running {
            app_id: app_id.to_string(),
            pid,
            unit: active.then_some(unit),
            uptime: SystemTime::now().duration_since(since).unwrap_or_default(),
        });
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.uptime));
    out
}

/// The memory it uses (its scope's), in bytes.
pub fn memory(r: &Running) -> Option<u64> {
    let unit = r.unit.as_ref()?;
    unit_property(unit, "MemoryCurrent")?.parse().ok()
}

/// Stop it: its scope (every process it has), else the `xapp run` that
/// started it (bwrap dies with it).
pub fn stop(r: &Running) -> Result<()> {
    match &r.unit {
        Some(unit) => run_program("systemctl", &["--user", "stop", unit]),
        None => {
            let pid = r.pid.to_string();
            run_program("kill", &["-TERM", &pid])
                .map_err(|e| err(format!("cannot stop {} ({pid}): {e}", r.app_id)))
        }
    }
}

/// "3 s", "5 min", "2 h 10 min", "3 d".
pub fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86400 => format!("{} h {} min", s / 3600, s % 3600 / 60),
        _ => format!("{} d", s / 86400),
    }
}

/// "512 KB", "120 MB", "1.5 GB".
pub fn human_size(bytes: u64) -> String {
    const M: u64 = 1024 * 1024;
    const G: u64 = 1024 * M;
    match bytes {
        0..M => format!("{} KB", bytes / 1024),
        M..G => format!("{} MB", bytes / M),
        _ => format!("{:.1} GB", bytes as f64 / G as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            split("org.mozilla.firefox-4242"),
            Some(("org.mozilla.firefox", 4242))
        );
        assert_eq!(
            split("org.example.my-app-7"),
            Some(("org.example.my-app", 7))
        );
        assert_eq!(split("nonsense"), None);
        assert_eq!(split("-7"), None);
        assert_eq!(
            unit_name("org.example.my-app", 7),
            "app-xapp-org.example.my\\x2dapp-7.scope"
        );
    }

    #[test]
    fn for_people() {
        assert_eq!(human_duration(Duration::from_secs(5)), "5 s");
        assert_eq!(human_duration(Duration::from_secs(7800)), "2 h 10 min");
        assert_eq!(human_size(300 * 1024 * 1024), "300 MB");
        assert_eq!(human_size(3 * 512 * 1024 * 1024), "1.5 GB");
    }
}
