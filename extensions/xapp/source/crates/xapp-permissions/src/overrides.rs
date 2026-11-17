//! What the user added to one application by hand, after `xapp doctor`
//! showed what its sandbox refused: names on the session bus it may own,
//! talk to or see, and capabilities. Like `flatpak override`, but only ever
//! more than the manifest, for this user, and undone with
//! `xapp doctor <app> --reset`.
//!
//! File: ~/.local/state/xapps/overrides/<app id>, lines `own=<name>`,
//! `talk=<name>`, `see=<name>`, `capability=<id>`.

use std::fs;
use std::path::PathBuf;

use xapp_core::paths;
use xapp_core::util::{Result, err, read_kv, write_atomic};

/// The kinds of lines, and what they give.
pub const KINDS: &[(&str, &str)] = &[
    ("own", "own the name on the session bus"),
    ("talk", "talk to the name on the session bus"),
    ("see", "see the name on the session bus"),
    ("capability", "capability"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides {
    pub own: Vec<String>,
    pub talk: Vec<String>,
    pub see: Vec<String>,
    pub capabilities: Vec<String>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.own.is_empty()
            && self.talk.is_empty()
            && self.see.is_empty()
            && self.capabilities.is_empty()
    }

    /// Every entry as (kind, value), in file order of kinds.
    pub fn entries(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        for (kind, list) in [
            ("own", &self.own),
            ("talk", &self.talk),
            ("see", &self.see),
            ("capability", &self.capabilities),
        ] {
            out.extend(list.iter().map(|v| (kind, v.as_str())));
        }
        out
    }

    pub fn has(&self, kind: &str, value: &str) -> bool {
        self.entries()
            .iter()
            .any(|(k, v)| *k == kind && *v == value)
    }

    fn list_mut(&mut self, kind: &str) -> Option<&mut Vec<String>> {
        match kind {
            "own" => Some(&mut self.own),
            "talk" => Some(&mut self.talk),
            "see" => Some(&mut self.see),
            "capability" => Some(&mut self.capabilities),
            _ => None,
        }
    }

    /// From lines `kind=value` (unknown kinds and bad values left out).
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Overrides {
        let mut o = Overrides::default();
        for (k, v) in pairs {
            if valid(&k, &v)
                && let Some(list) = o.list_mut(&k)
                && !list.contains(&v)
            {
                list.push(v);
            }
        }
        o
    }

    pub fn to_text(&self) -> String {
        self.entries()
            .iter()
            .map(|(k, v)| format!("{k}={v}\n"))
            .collect()
    }
}

/// A well-known name on the session bus (org.example.Name): not a unique
/// name (":1.42"), not the bus itself.
pub fn valid_bus_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.contains('.')
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name != "org.freedesktop.DBus"
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && name
            .split('.')
            .all(|part| !part.starts_with(|c: char| c.is_ascii_digit()))
}

fn valid(kind: &str, value: &str) -> bool {
    match kind {
        "own" | "talk" | "see" => valid_bus_name(value),
        "capability" => crate::capability(value).is_some(),
        _ => false,
    }
}

fn file(app_id: &str) -> PathBuf {
    paths::state().join("overrides").join(app_id)
}

/// What the user added for an application (nothing: empty).
pub fn load(app_id: &str) -> Overrides {
    Overrides::from_pairs(read_kv(&file(app_id)))
}

/// Add one entry.
pub fn add(app_id: &str, kind: &str, value: &str) -> Result<()> {
    if !valid(kind, value) {
        return Err(err(format!("not a valid {kind}: \"{value}\"")));
    }
    let mut o = load(app_id);
    if let Some(list) = o.list_mut(kind)
        && !list.iter().any(|v| v == value)
    {
        list.push(value.to_string());
    }
    write_atomic(&file(app_id), o.to_text().as_bytes())
}

/// Forget everything added for an application.
pub fn reset(app_id: &str) {
    let _ = fs::remove_file(file(app_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_bus_name("org.mpris.MediaPlayer2.vlc"));
        assert!(valid_bus_name("org.kde.StatusNotifierItem-2-1"));
        assert!(!valid_bus_name(":1.42"));
        assert!(!valid_bus_name("org.freedesktop.DBus"));
        assert!(!valid_bus_name("nodot"));
        assert!(!valid_bus_name("org..x"));
        assert!(!valid_bus_name("org.1x"));
        assert!(!valid_bus_name("org.x;rm"));
    }

    #[test]
    fn parse_and_write() {
        let pairs = vec![
            ("own".to_string(), "org.mpris.MediaPlayer2.vlc".to_string()),
            (
                "talk".to_string(),
                "org.kde.StatusNotifierWatcher".to_string(),
            ),
            (
                "talk".to_string(),
                "org.kde.StatusNotifierWatcher".to_string(),
            ),
            ("capability".to_string(), "x11".to_string()),
            ("capability".to_string(), "everything".to_string()),
            ("root".to_string(), "yes".to_string()),
        ];
        let o = Overrides::from_pairs(pairs);
        assert_eq!(o.own, ["org.mpris.MediaPlayer2.vlc"]);
        assert_eq!(o.talk.len(), 1);
        assert_eq!(o.capabilities, ["x11"]);
        assert!(o.has("capability", "x11"));
        let again = Overrides::from_pairs(
            o.to_text()
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        assert_eq!(again, o);
        assert!(Overrides::default().is_empty());
    }
}
