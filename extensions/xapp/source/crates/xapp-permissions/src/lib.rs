//! Permissions: what an application may do beyond running in its sandbox,
//! like the TCC of macOS.
//!
//! The application declares in its manifest the permissions it may ask for,
//! each with a reason ("Firefox uses the microphone for video calls"). A
//! permission it did not declare can never be given. The user decides the
//! others: some are given by default (network, graphics, notifications, for
//! an application from a trusted or approved author), the rest are asked
//! when the application starts, and every choice can be changed with
//! `xapp permissions`. What is not given does not exist in the sandbox: a
//! folder not given is not there at all.
//!
//! Choices: ~/.local/state/xapps/permissions/<app id>, lines `<id>=yes|no`;
//! for every application, ~/.local/state/xapps/permissions.all. Like
//! `flatpak override`, the user has the last word: a choice for one
//! application wins over one for all, and both win over the manifest. A
//! permission the manifest does not declare can be added by hand
//! (`xapp permissions <app> <id> yes`, or `--all`): it is then shown as
//! "added by you".
//!
//! Two levels, as on macOS (entitlements of the developer, TCC of the user):
//! permissions are about the user's things (folders, camera, microphone,
//! network) and are the user's to give; capabilities (CAPABILITIES) are what
//! the application needs from the system to work at all, are found in its
//! image by `xapp build`, and are given without asking. A few more things
//! every application has (its own names on the session bus, its media
//! player name, the tray, the parts of the desktop portal that ask the user
//! themselves): see the sandbox (dbus.rs). What the user adds by hand after
//! `xapp doctor` is in `overrides`.
//!
//! Besides the catalog, custom permissions, like Flatpak's: a name on the
//! session bus (`dbus-talk:<name>`, `dbus-own:<name>`), a folder
//! (`folder:~/Games`, `folder:/mnt/data:ro`), devices (`device:input`,
//! `device:all`). Unlike Flatpak they are not given in silence: declared
//! with a reason and asked like the others (Custom).

pub mod overrides;

use std::fs;
use std::path::PathBuf;

use xapp_core::paths;
use xapp_core::util::{Result, err, read_kv, write_atomic};
use xapp_format::{Manifest, Permission};

/// A known permission.
pub struct Info {
    pub id: &'static str,
    /// What it gives, for people.
    pub label: &'static str,
    /// Given without asking (to applications whose author is trusted or
    /// approved), unless the user said no.
    pub default: bool,
    /// For folders: the XDG name of the folder (DOCUMENTS, DOWNLOAD, ...)
    /// and the English default.
    pub folder: Option<(&'static str, &'static str)>,
}

pub const CATALOG: &[Info] = &[
    Info {
        id: "network",
        label: "Network and internet",
        default: true,
        folder: None,
    },
    Info {
        id: "gpu",
        label: "Graphics acceleration (GPU)",
        default: true,
        folder: None,
    },
    Info {
        id: "notifications",
        label: "Notifications",
        default: true,
        folder: None,
    },
    Info {
        id: "documents",
        label: "Your Documents folder",
        default: false,
        folder: Some(("DOCUMENTS", "Documents")),
    },
    Info {
        id: "downloads",
        label: "Your Downloads folder",
        default: false,
        folder: Some(("DOWNLOAD", "Downloads")),
    },
    Info {
        id: "pictures",
        label: "Your Pictures folder",
        default: false,
        folder: Some(("PICTURES", "Pictures")),
    },
    Info {
        id: "music",
        label: "Your Music folder",
        default: false,
        folder: Some(("MUSIC", "Music")),
    },
    Info {
        id: "videos",
        label: "Your Videos folder",
        default: false,
        folder: Some(("VIDEOS", "Videos")),
    },
    Info {
        id: "desktop",
        label: "Your Desktop folder",
        default: false,
        folder: Some(("DESKTOP", "Desktop")),
    },
    Info {
        id: "audio",
        label: "Sound and microphone",
        default: false,
        folder: None,
    },
    Info {
        id: "camera",
        label: "Camera",
        default: false,
        folder: None,
    },
    Info {
        id: "autostart",
        label: "Start when you log in",
        default: false,
        folder: None,
    },
];

/// A capability: needed from the system to work, given without asking.
pub struct Capability {
    pub id: &'static str,
    /// What it gives, for people.
    pub label: &'static str,
}

pub const CAPABILITIES: &[Capability] = &[Capability {
    id: "x11",
    label: "X11 windows (an interface that opens only on X11; less isolated from other X11 windows)",
}];

/// A capability's description, if it is a known one.
pub fn capability(id: &str) -> Option<&'static Capability> {
    CAPABILITIES.iter().find(|c| c.id == id)
}

/// Does the application have a capability (from its file, or added by the
/// user with `xapp doctor`)?
pub fn has_capability(m: &Manifest, id: &str) -> bool {
    capability(id).is_some()
        && (m.capabilities.iter().any(|c| c == id)
            || overrides::load(&m.app_id).has("capability", id))
}

/// A permission's description, if it is a known one.
pub fn info(id: &str) -> Option<&'static Info> {
    CATALOG.iter().find(|i| i.id == id)
}

/// A custom permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Custom {
    /// Talking to a name on the session bus.
    Talk(String),
    /// Owning a name on the session bus.
    Own(String),
    /// A folder: `~/...` (in the home) or under /mnt, /media, /run/media.
    Folder { path: String, read_only: bool },
    /// Devices: `input` (game controllers: /dev/input, hidraw, uinput) or
    /// `all` (every device of /dev).
    Device(String),
}

/// The kinds of custom permissions, for people.
pub const CUSTOM_KINDS: &[(&str, &str)] = &[
    (
        "dbus-talk",
        "talk to a name on the session bus (like Flatpak's --talk-name)",
    ),
    (
        "dbus-own",
        "own a name on the session bus (like Flatpak's --own-name)",
    ),
    (
        "folder",
        "a folder: ~/Games, /mnt/data (add :ro for read-only)",
    ),
    ("device", "devices: input (game controllers) or all"),
];

/// Names on the session bus that give a lot: asked with "no" first, and
/// never given by --yes.
const SENSITIVE: &[(&str, &str)] = &[
    ("org.freedesktop.secrets", "every password in your keyring"),
    ("org.kde.kwalletd", "every password in your wallet"),
    ("org.gnome.keyring", "every password in your keyring"),
    ("org.freedesktop.systemd1", "your services"),
    (
        "org.freedesktop.Flatpak",
        "running programs outside any sandbox",
    ),
    (
        "org.freedesktop.impl.portal",
        "the desktop portal's insides, without its questions",
    ),
    ("org.gnome.Shell", "your desktop"),
    ("org.kde.KWin", "your windows and screen"),
    ("org.kde.plasmashell", "your desktop"),
    ("org.kde.klauncher", "starting programs outside any sandbox"),
    ("org.freedesktop.ScreenSaver", "locking your screen"),
];

/// What a sensitive bus name gives, if it is one (or, for a prefix
/// wildcard `a.b.c.*`, if it covers one).
pub fn sensitive_bus_name(name: &str) -> Option<&'static str> {
    let stem = name.strip_suffix(".*");
    SENSITIVE
        .iter()
        .find(|(prefix, _)| name.starts_with(prefix) || stem.is_some_and(|s| prefix.starts_with(s)))
        .map(|(_, what)| *what)
}

/// A folder of a custom permission, checked: in the home (not the home
/// itself, not a hidden folder at its top: ~/.ssh, ~/.config...) or under
/// /mnt, /media, /run/media.
fn valid_folder(path: &str) -> bool {
    let ok_chars =
        !path.contains("..") && !path.contains(':') && !path.contains('\n') && !path.ends_with('/');
    if let Some(rest) = path.strip_prefix("~/") {
        // Not "~//home/x": joined to the home, an absolute rest replaces it.
        return ok_chars && !rest.is_empty() && !rest.starts_with(['.', '/']);
    }
    ok_chars
        && ["/mnt/", "/media/", "/run/media/"]
            .iter()
            .any(|p| path.strip_prefix(p).is_some_and(|r| !r.is_empty()))
}

/// A custom permission, if `id` is one (and well formed).
pub fn custom(id: &str) -> Option<Custom> {
    let (kind, value) = id.split_once(':')?;
    match kind {
        "dbus-talk" if overrides::valid_bus_pattern(value) => Some(Custom::Talk(value.into())),
        "dbus-own" if overrides::valid_bus_pattern(value) => Some(Custom::Own(value.into())),
        "folder" => {
            let (path, read_only) = match value.strip_suffix(":ro") {
                Some(p) => (p, true),
                None => (value.strip_suffix(":rw").unwrap_or(value), false),
            };
            valid_folder(path).then(|| Custom::Folder {
                path: path.into(),
                read_only,
            })
        }
        "device" if matches!(value, "input" | "all") => Some(Custom::Device(value.into())),
        _ => None,
    }
}

impl Custom {
    /// The permission's id, as written.
    pub fn id(&self) -> String {
        match self {
            Custom::Talk(n) => format!("dbus-talk:{n}"),
            Custom::Own(n) => format!("dbus-own:{n}"),
            Custom::Folder { path, read_only } => {
                format!("folder:{path}{}", if *read_only { ":ro" } else { "" })
            }
            Custom::Device(d) => format!("device:{d}"),
        }
    }

    /// What it gives, for people.
    pub fn label(&self) -> String {
        match self {
            Custom::Talk(n) => match n.strip_suffix(".*") {
                Some(stem) => format!("Talk to {stem} and the names under it (session bus)"),
                None => format!("Talk to {n} (session bus)"),
            },
            Custom::Own(n) => match n.strip_suffix(".*") {
                Some(stem) => format!("Take the name {stem} and those under it (session bus)"),
                None => format!("Take the name {n} (session bus)"),
            },
            Custom::Folder { path, read_only } => format!(
                "The folder {path}{}",
                if *read_only { " (read only)" } else { "" }
            ),
            Custom::Device(d) if d == "input" => "Input devices (game controllers)".to_string(),
            Custom::Device(_) => "Every device of the computer".to_string(),
        }
    }

    /// Why it is to be given with care (asked with "no" first).
    pub fn warning(&self) -> Option<String> {
        match self {
            Custom::Talk(n) | Custom::Own(n) => {
                sensitive_bus_name(n).map(|what| format!("It gives {what}."))
            }
            Custom::Device(d) if d == "all" => {
                Some("It gives every device: disks, keyboards, other users' devices.".into())
            }
            _ => None,
        }
    }

    /// A folder's real path (`~` is the user's home).
    pub fn folder_path(&self) -> Option<(PathBuf, bool)> {
        match self {
            Custom::Folder { path, read_only } => {
                let p = match path.strip_prefix("~/") {
                    Some(rest) => paths::home().join(rest),
                    None => PathBuf::from(path),
                };
                Some((p, *read_only))
            }
            _ => None,
        }
    }
}

/// A permission's label, for people: known or custom.
pub fn label(id: &str) -> String {
    match (info(id), custom(id)) {
        (Some(i), _) => i.label.to_string(),
        (None, Some(c)) => c.label(),
        _ => id.to_string(),
    }
}

/// Is it a permission (known or custom)?
pub fn valid(id: &str) -> bool {
    info(id).is_some() || custom(id).is_some()
}

/// Given with care: asked with "no" first, never given by --yes.
pub fn warning(id: &str) -> Option<String> {
    custom(id).and_then(|c| c.warning())
}

/// The state of one permission for one application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Yes,
    No,
    /// Not decided yet: to ask at the next start.
    Ask,
}

fn file(app_id: &str) -> PathBuf {
    paths::state().join("permissions").join(app_id)
}

/// The choices for every application.
fn all_file() -> PathBuf {
    paths::state().join("permissions.all")
}

/// The choices of a file, the last one for each permission, in order.
fn choices_in(path: &std::path::Path) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for (k, v) in read_kv(path) {
        out.retain(|(id, _)| *id != k);
        out.push((k, v == "yes"));
    }
    out
}

fn decided_in(path: &std::path::Path, id: &str) -> Option<bool> {
    choices_in(path)
        .into_iter()
        .find(|(k, _)| k == id)
        .map(|(_, v)| v)
}

/// What the user decided for this application, else for every application
/// (`None`: nothing yet).
pub fn decided(app_id: &str, id: &str) -> Option<bool> {
    decided_in(&file(app_id), id).or_else(|| decided_in(&all_file(), id))
}

/// The choices for every application (`xapp permissions --all`).
pub fn global_choices() -> Vec<(String, bool)> {
    choices_in(&all_file())
}

fn set_in(path: &std::path::Path, id: &str, value: Option<bool>) -> Result<()> {
    if !valid(id) {
        return Err(err(format!(
            "unknown permission \"{id}\" (known: {})",
            known()
        )));
    }
    let mut entries: Vec<(String, String)> =
        read_kv(path).into_iter().filter(|(k, _)| k != id).collect();
    if let Some(v) = value {
        entries.push((id.to_string(), if v { "yes" } else { "no" }.to_string()));
    }
    let text: String = entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    write_atomic(path, text.as_bytes())
}

/// Record a choice (`None`: forget it, ask again). `yes` on a permission
/// the manifest does not declare adds it.
pub fn set(app_id: &str, id: &str, value: Option<bool>) -> Result<()> {
    set_in(&file(app_id), id, value)
}

/// Record a choice for every application (`None`: forget it).
pub fn set_global(id: &str, value: Option<bool>) -> Result<()> {
    set_in(&all_file(), id, value)
}

/// Forget all the choices for an application.
pub fn forget(app_id: &str) {
    let _ = fs::remove_file(file(app_id));
}

/// The reason shown for a permission the user added by hand.
pub const ADDED: &str = "added by you";

/// The permissions the user added to the ones the manifest declares: given
/// by hand to this application or to every one (and not taken back for
/// this one).
pub fn added(m: &Manifest) -> Vec<Permission> {
    added_from(
        &m.permissions,
        &choices_in(&file(&m.app_id)),
        &global_choices(),
    )
}

fn added_from(
    declared: &[Permission],
    app: &[(String, bool)],
    global: &[(String, bool)],
) -> Vec<Permission> {
    let mut out: Vec<Permission> = Vec::new();
    for (id, _) in app.iter().chain(global).filter(|(_, yes)| *yes) {
        let own = app.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
        if own != Some(false)
            && valid(id)
            && !declared.iter().any(|p| p.id == *id)
            && !out.iter().any(|p| p.id == *id)
        {
            out.push(Permission {
                id: id.clone(),
                reason: ADDED.to_string(),
            });
        }
    }
    out
}

/// The declared permissions and those the user added.
pub fn effective(m: &Manifest) -> Vec<Permission> {
    let mut all = m.permissions.clone();
    all.extend(added(m));
    all
}

/// The names of all known permissions, for messages.
pub fn known() -> String {
    format!(
        "{}; custom: dbus-talk:<name>, dbus-own:<name>, folder:<~/path or /mnt/...>[:ro], device:input|all",
        CATALOG.iter().map(|i| i.id).collect::<Vec<_>>().join(", ")
    )
}

/// The state of a permission. `known_author`: trusted or approved author
/// (defaults apply only then).
pub fn state(m: &Manifest, id: &str, known_author: bool) -> State {
    let default = match (info(id), custom(id)) {
        (Some(i), _) => i.default,
        (None, Some(_)) => false,
        _ => return State::No,
    };
    resolve(
        m.declares(id).is_some(),
        default && known_author,
        decided(&m.app_id, id),
    )
}

/// The state from: declared by the manifest, given by default, what the
/// user decided. Undeclared: only what the user added by hand.
fn resolve(declared: bool, default: bool, decided: Option<bool>) -> State {
    match (decided, declared) {
        (Some(true), _) => State::Yes,
        (Some(false), _) | (None, false) => State::No,
        (None, true) if default => State::Yes,
        (None, true) => State::Ask,
    }
}

/// Is the permission given?
pub fn granted(m: &Manifest, id: &str, known_author: bool) -> bool {
    state(m, id, known_author) == State::Yes
}

/// The declared permissions still to ask about: the catalog's in its
/// order, then the custom ones.
pub fn pending(m: &Manifest, known_author: bool) -> Vec<&Permission> {
    CATALOG
        .iter()
        .filter_map(|i| m.declares(i.id))
        .chain(m.permissions.iter().filter(|p| custom(&p.id).is_some()))
        .filter(|p| state(m, &p.id, known_author) == State::Ask)
        .collect()
}

/// The custom permissions given.
pub fn granted_custom(m: &Manifest, known_author: bool) -> Vec<Custom> {
    effective(m)
        .iter()
        .filter_map(|p| custom(&p.id))
        .filter(|c| granted(m, &c.id(), known_author))
        .collect()
}

/// Every declared permission, then those the user added, with its state
/// (unknown ones: never given).
pub fn all(m: &Manifest, known_author: bool) -> Vec<(Permission, State)> {
    effective(m)
        .into_iter()
        .map(|p| {
            let s = state(m, &p.id, known_author);
            (p, s)
        })
        .collect()
}

/// The real path of a user folder (from ~/.config/user-dirs.dirs, which
/// holds the translated names: ~/Documenti in Italian).
pub fn folder_path(id: &str) -> Option<PathBuf> {
    let (xdg, default) = info(id)?.folder?;
    let home = paths::home();
    let dirs = fs::read_to_string(paths::config_home().join("user-dirs.dirs")).unwrap_or_default();
    let key = format!("XDG_{xdg}_DIR=");
    let from_file = dirs.lines().find_map(|l| {
        let value = l.trim().strip_prefix(&key)?.trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            PathBuf::from(format!("{}{rest}", home.display()))
        } else {
            PathBuf::from(value)
        };
        // A folder set to the home itself means "none" for xdg-user-dirs.
        (path.is_absolute() && path != home).then_some(path)
    });
    Some(from_file.unwrap_or_else(|| home.join(default)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        for (n, i) in CATALOG.iter().enumerate() {
            assert!(
                CATALOG[n + 1..].iter().all(|j| j.id != i.id),
                "duplicate {}",
                i.id
            );
            assert!(
                !(i.default && i.folder.is_some()),
                "a folder is never given by default"
            );
        }
        assert!(info("camera").is_some());
        // A capability, not a permission.
        assert!(info("x11").is_none());
        assert!(capability("x11").is_some());
        for c in CAPABILITIES {
            assert!(info(c.id).is_none(), "{} is both", c.id);
        }
        // Not a permission: decided by the package (manifest user_namespaces).
        assert!(info("nested_sandbox").is_none());
        assert!(info("everything").is_none());
    }

    #[test]
    fn custom_permissions() {
        assert_eq!(
            custom("dbus-talk:org.kde.kwalletd6"),
            Some(Custom::Talk("org.kde.kwalletd6".into()))
        );
        assert!(warning("dbus-talk:org.kde.kwalletd6").is_some());
        assert!(warning("dbus-own:org.example.Game").is_none());
        assert_eq!(
            custom("dbus-own:com.example.Game.Instance.*"),
            Some(Custom::Own("com.example.Game.Instance.*".into()))
        );
        assert!(warning("dbus-own:com.example.Game.*").is_none());
        assert!(warning("dbus-talk:org.freedesktop.impl.*").is_some());
        assert!(custom("dbus-own:org.kde.*").is_none());
        assert!(custom("dbus-own:*").is_none());
        assert_eq!(
            custom("folder:~/Giochi:ro"),
            Some(Custom::Folder {
                path: "~/Giochi".into(),
                read_only: true
            })
        );
        assert_eq!(
            custom("folder:~/Giochi:ro").unwrap().id(),
            "folder:~/Giochi:ro"
        );
        assert!(custom("folder:/mnt/dati").is_some());
        assert!(custom("device:input").is_some());
        assert!(warning("device:all").is_some());
        for bad in [
            "folder:~",
            "folder:~/",
            "folder:~/.ssh",
            "folder:~//home/alice/.ssh",
            "folder:/",
            "folder:/etc",
            "folder:/mnt/../etc",
            "folder:~/a/../../b",
            "device:gpu",
            "dbus-talk:org.freedesktop.DBus",
            "dbus-talk::1.42",
            "camera:x",
            "nonsense",
        ] {
            assert!(custom(bad).is_none(), "{bad}");
        }
        assert!(valid("camera") && valid("device:input") && !valid("device:x"));
        assert_eq!(label("folder:/mnt/dati"), "The folder /mnt/dati");
    }

    #[test]
    fn the_user_has_the_last_word() {
        // Declared: asked, given by default, or what the user said.
        assert_eq!(resolve(true, false, None), State::Ask);
        assert_eq!(resolve(true, true, None), State::Yes);
        assert_eq!(resolve(true, true, Some(false)), State::No);
        // Not declared: never, unless added by hand.
        assert_eq!(resolve(false, true, None), State::No);
        assert_eq!(resolve(false, false, Some(true)), State::Yes);
    }

    #[test]
    fn permissions_added_by_hand() {
        let m = [Permission {
            id: "network".into(),
            reason: "r".into(),
        }];
        let yes = |id: &str| (id.to_string(), true);
        let no = |id: &str| (id.to_string(), false);
        let ids = |v: Vec<Permission>| v.into_iter().map(|p| p.id).collect::<Vec<_>>();
        // For this one, for all; declared ones and unknown ones are not added.
        assert_eq!(
            ids(added_from(
                &m,
                &[yes("camera"), yes("network"), yes("nonsense")],
                &[yes("folder:~/Giochi"), yes("camera"), no("audio")]
            )),
            ["camera", "folder:~/Giochi"]
        );
        // Given to all, taken back for this one.
        assert!(added_from(&m, &[no("camera")], &[yes("camera")]).is_empty());
    }
}
