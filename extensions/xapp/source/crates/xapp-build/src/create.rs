//! `xapp create`: a recipe made for you (the wizard).
//!
//! From a package of the Arch Linux Archive (`--arch <package>`) it finds,
//! without installing anything: the name, the description and the version;
//! the menu entry of the package (.desktop), and from it the command, the
//! icon, the categories, the file types and the window name; and every
//! package that installing it brings, from which it proposes the
//! permissions (PipeWire: sound; libcamera: camera; ...). The user confirms
//! each permission; the result is a recipe directory, `recipe.json`, to read,
//! keep (xeeonos-applications) and build with `xapp build`.
//!
//! From an archive the application's authors publish (`--url https://...`,
//! e.g. Firefox from mozilla.org) it downloads it, records its SHA-256,
//! finds the program to start and the icon, and the packages of the
//! libraries its programs need (and, if the user wants, of those they load
//! while running); the archive goes into /opt/<name> of the image (recipe
//! `sources`).
//!
//! From a directory of the user's (`--dir <directory>`: a program built at
//! home, or an archive already unpacked) the same as from an archive,
//! without the download: the recipe's source is that directory.
//!
//! From the AUR (`--aur <package>`) it fetches the package's PKGBUILD and
//! shows it: it is code written by anyone, and is built only if the user
//! says so (`--pkgbuild-ok` for scripts: `--yes` alone does not). Then it
//! is built (makepkg, as a normal user, in a root made from the snapshot,
//! inside the container) and looked at like a package of the snapshot; the
//! recipe pins the commit of the PKGBUILD that was read (recipe `aur`).
//!
//! All are looked at in an Arch Linux container (create-arch.sh,
//! create-url.sh, create-aur.sh with aur.sh), like `xapp build` does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use xapp_core::paths;
use xapp_core::ui::{self, Question};
use xapp_core::util::{Result, err, gid, uid, valid_app_id};

const ARCH_SCRIPT: &str = include_str!("create-arch.sh");
const URL_SCRIPT: &str = include_str!("create-url.sh");
const AUR_SCRIPT: &str = include_str!("create-aur.sh");
/// Shared by the scripts above: what the files of an application say.
const SCAN_LIB: &str = include_str!("scan.sh");

/// Where the application comes from.
pub enum Source {
    /// A package of the Arch Linux Archive snapshot.
    Arch(String),
    /// An archive published by the application's authors (https://...).
    Url(String),
    /// A directory of the user's with the application's files.
    Dir(PathBuf),
    /// A package of the AUR, built from its PKGBUILD.
    Aur(String),
}

/// Options of `xapp create`.
pub struct Options {
    pub source: Source,
    /// YYYY/MM/DD; default: the snapshot of this XeeonOS.
    pub snapshot: Option<String>,
    /// The application id, if the package's own is not one.
    pub app_id: Option<String>,
    /// The recipe directory; default: ./<package>.
    pub output: Option<PathBuf>,
    pub work: Option<PathBuf>,
    /// `--url`: the program to start and the icon, as paths inside the
    /// archive, when the wizard cannot tell.
    pub exec: Option<String>,
    pub icon: Option<String>,
    /// `--aur`: build the PKGBUILD without asking (scripts that read it
    /// some other way).
    pub pkgbuild_ok: bool,
    /// `--permission <id>[=<reason>]`: ticked in the list from the start.
    pub permissions: Vec<String>,
}

/// What a .desktop file says (its [Desktop Entry] group).
#[derive(Debug, Default, PartialEq)]
pub struct Desktop {
    pub name: String,
    pub names: Vec<(String, String)>,
    pub comment: String,
    pub exec: String,
    pub icon: String,
    pub categories: Vec<String>,
    pub mime_types: Vec<String>,
    pub wm_class: String,
    pub hidden: bool,
}

/// Read a .desktop file.
pub fn parse_desktop(text: &str) -> Desktop {
    let mut d = Desktop::default();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim().to_string());
        let list = |v: &str| -> Vec<String> {
            v.split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        };
        match k {
            "Name" => d.name = v,
            "Comment" => d.comment = v,
            "Exec" => d.exec = v,
            "Icon" => d.icon = v,
            "Categories" => d.categories = list(&v),
            "MimeType" => d.mime_types = list(&v),
            "StartupWMClass" => d.wm_class = v,
            "NoDisplay" | "Hidden" if v == "true" => d.hidden = true,
            _ => {
                // Name[it]: only plain languages (it, de), not it_IT@euro.
                if let Some(lang) = k.strip_prefix("Name[").and_then(|l| l.strip_suffix(']'))
                    && lang.len() == 2
                    && lang.chars().all(|c| c.is_ascii_lowercase())
                {
                    d.names.push((lang.to_string(), v));
                }
            }
        }
    }
    d
}

/// The words of an Exec line, without the field codes (%U, %f, ...) and a
/// leading `env VAR=value`.
pub fn exec_words(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in exec.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(cur);
    }
    let mut words: Vec<String> = words
        .into_iter()
        .filter(|w| !(w.len() == 2 && w.starts_with('%')))
        .collect();
    if words
        .first()
        .is_some_and(|w| w == "env" || w.ends_with("/env"))
    {
        words.remove(0);
        while words
            .first()
            .is_some_and(|w| w.contains('=') && !w.starts_with('-'))
        {
            words.remove(0);
        }
    }
    words
}

/// The command as `xapp` needs it: an absolute path first. `files`: the
/// package's files (usr/bin/...).
pub fn command(exec: &str, files: &[String]) -> Option<Vec<String>> {
    let mut words = exec_words(exec);
    let first = words.first()?.clone();
    if !first.starts_with('/') {
        let found = ["usr/bin/", "usr/lib/", "opt/"]
            .iter()
            .find_map(|dir| {
                files
                    .iter()
                    .find(|f| f.starts_with(dir) && f.rsplit('/').next() == Some(first.as_str()))
            })
            .cloned()
            .unwrap_or_else(|| format!("usr/bin/{first}"));
        words[0] = format!("/{found}");
    }
    Some(words)
}

/// The icon file inside the image (starting with /) for an Icon= name:
/// the scalable one, else the biggest PNG of hicolor, else pixmaps.
pub fn icon_path(icon: &str, files: &[String]) -> Option<String> {
    if icon.starts_with('/') {
        return Some(icon.to_string());
    }
    let svg = format!("usr/share/icons/hicolor/scalable/apps/{icon}.svg");
    if files.contains(&svg) {
        return Some(format!("/{svg}"));
    }
    let suffix = format!("/apps/{icon}.png");
    let best = files
        .iter()
        .filter(|f| f.starts_with("usr/share/icons/hicolor/") && f.ends_with(&suffix))
        .filter_map(|f| {
            let size = f
                .strip_prefix("usr/share/icons/hicolor/")?
                .split('/')
                .next()?
                .split('x')
                .next()?
                .parse::<u32>()
                .ok()?;
            Some((size, f))
        })
        .max_by_key(|(size, _)| *size);
    if let Some((_, f)) = best {
        return Some(format!("/{f}"));
    }
    ["svg", "png"].iter().find_map(|ext| {
        let p = format!("usr/share/pixmaps/{icon}.{ext}");
        files.contains(&p).then(|| format!("/{p}"))
    })
}

/// A menu icon the package does not have, from an icon theme of the
/// snapshot: `candidates` are "package path" lines (create-arch.sh,
/// icons.txt). Prefers a package already in the closure, then hicolor, then
/// SVG, then the biggest size. Returns (package, path).
pub fn theme_icon(
    icon: &str,
    candidates: &[String],
    closure: &[String],
) -> Option<(String, String)> {
    let size = |path: &str| -> u32 {
        path.split('/')
            .filter_map(|part| part.split(['x', '@']).next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
    };
    candidates
        .iter()
        .filter_map(|l| {
            let (pkg, path) = l.split_once(' ')?;
            let name = path.rsplit('/').next()?;
            let ext = name.strip_prefix(icon)?.strip_prefix('.')?;
            matches!(ext, "png" | "svg").then_some((pkg, path, ext))
        })
        .max_by_key(|(pkg, path, ext)| {
            (
                closure.iter().any(|c| c == pkg),
                path.contains("/hicolor/"),
                !path.contains("/symbolic/") && !path.ends_with("-symbolic.svg"),
                *ext == "svg",
                size(path),
            )
        })
        .map(|(pkg, path, _)| {
            (
                pkg.to_string(),
                format!("/{}", path.trim_start_matches('/')),
            )
        })
}

/// A permission proposed by the wizard, with the reason it gives.
#[derive(Debug, PartialEq)]
pub struct Proposal {
    pub id: &'static str,
    pub reason: String,
    /// Why the wizard thinks so (shown to the author, not in the recipe).
    pub because: String,
}

/// The permissions an application probably needs, from the packages it
/// brings (`closure`), its categories, the file types it opens, its own
/// files (a web engine inside: Electron, Chromium, CEF, WebKit, Qt
/// WebEngine, Firefox) and what its programs name (`hints`, from
/// create-url.sh: "camera /dev/video", "audio libpulse.so.0").
pub fn propose(
    closure: &[String],
    categories: &[String],
    mime: &[String],
    files: &[String],
    hints: &[String],
) -> Vec<Proposal> {
    let has = |names: &[&str]| -> Option<String> {
        closure
            .iter()
            .find(|p| {
                names
                    .iter()
                    .any(|n| p == n || (n.ends_with('*') && p.starts_with(&n[..n.len() - 1])))
            })
            .cloned()
    };
    let cat = |c: &str| categories.iter().any(|x| x == c);
    let mime_any = |prefixes: &[&str]| {
        mime.iter()
            .any(|m| prefixes.iter().any(|p| m.starts_with(p)))
    };
    let mut out: Vec<Proposal> = Vec::new();
    let mut add = |id: &'static str, reason: &str, because: String| {
        if !out.iter().any(|p| p.id == id) {
            out.push(Proposal {
                id,
                reason: reason.to_string(),
                because,
            })
        }
    };
    let net = has(&[
        "curl",
        "libsoup3",
        "libsoup",
        "glib-networking",
        "nss",
        "webkit*",
        "qt6-webengine",
        "qt5-webengine",
        "electron*",
    ]);
    if cat("Network") || net.is_some() {
        add(
            "network",
            "To connect to the internet",
            net.map_or("category Network".into(), |p| format!("uses {p}")),
        );
    }
    if let Some(p) = has(&[
        "mesa",
        "libglvnd",
        "vulkan-icd-loader",
        "gtk4",
        "qt6-base",
        "qt5-base",
        "sdl2",
        "sdl2-compat",
        "sdl3",
    ]) {
        add("gpu", "To draw its windows smoothly", format!("uses {p}"));
    }
    if let Some(p) = has(&["libnotify", "gtk4", "libadwaita"]) {
        add(
            "notifications",
            "To show notifications",
            format!("uses {p}"),
        );
    }
    if let Some(p) = has(&[
        "libpulse",
        "pipewire",
        "alsa-lib",
        "gst-plugins-base-libs",
        "sdl2",
        "sdl2-compat",
        "sdl3",
        "openal",
    ]) {
        add("audio", "To play sound", format!("uses {p}"));
    }
    if let Some(p) = has(&["libcamera", "v4l-utils", "libv4l"]) {
        add("camera", "To use the camera", format!("uses {p}"));
    }
    if mime_any(&[
        "text/",
        "application/pdf",
        "application/vnd.",
        "application/x-tex",
        "application/rtf",
    ]) {
        add(
            "documents",
            "To open and save your documents",
            "opens documents".into(),
        );
    }
    if mime_any(&["image/"]) {
        add(
            "pictures",
            "To open and save your pictures",
            "opens images".into(),
        );
    }
    if mime_any(&["audio/"]) {
        add("music", "To open your music", "opens audio files".into());
    }
    if mime_any(&["video/"]) {
        add("videos", "To open your videos", "opens videos".into());
    }
    if cat("WebBrowser") || cat("FileTransfer") || cat("P2P") {
        add(
            "downloads",
            "To save the files you download",
            "downloads files".into(),
        );
    }
    // A web engine inside: its pages can make audio and video calls
    // (WebRTC), whatever the packages say (Electron brings its own).
    let engine = crate::own_sandbox(files.iter().map(String::as_str))
        .map(|(_, what)| what.to_string())
        .or_else(|| {
            files
                .iter()
                .any(|f| f.ends_with("/libxul.so") || f == "libxul.so")
                .then(|| "Firefox's engine".to_string())
        })
        .or_else(|| {
            has(&[
                "electron*",
                "qt6-webengine",
                "qt5-webengine",
                "webkit2gtk*",
                "webkitgtk*",
                "cef*",
            ])
        });
    if let Some(e) = &engine {
        let because = format!("a web engine inside ({e}): its pages can make calls");
        add("audio", "For audio and video calls", because.clone());
        add("camera", "For video calls", because.clone());
        add("notifications", "To show notifications", because);
    }
    if cat("InstantMessaging") || cat("Chat") || cat("VideoConference") || cat("Telephony") {
        add("audio", "For calls", "a chat or calls application".into());
        add(
            "camera",
            "For video calls",
            "a chat or calls application".into(),
        );
        add(
            "notifications",
            "To tell you about new messages",
            "a chat application".into(),
        );
    }
    if cat("Audio") || cat("Music") || cat("Player") || cat("Recorder") || cat("Video") {
        add(
            "audio",
            "To play sound",
            "a sound or video application".into(),
        );
    }
    for h in hints {
        let (id, why) = h.split_once(' ').unwrap_or((h.as_str(), ""));
        match id {
            "audio" => add("audio", "To play sound", format!("its programs name {why}")),
            "camera" => add(
                "camera",
                "To use the camera",
                format!("its programs name {why}"),
            ),
            _ => {}
        }
    }
    out
}

/// The reason shown to the user for a permission the author ticks without
/// a proposal.
pub fn default_reason(id: &str) -> String {
    match id {
        "network" => "To connect to the internet",
        "gpu" => "To draw its windows smoothly",
        "notifications" => "To show notifications",
        "documents" => "To open and save your documents",
        "downloads" => "To save the files you download",
        "pictures" => "To open and save your pictures",
        "music" => "To open your music",
        "videos" => "To open your videos",
        "desktop" => "To open the files on your desktop",
        "audio" => "To play sound and use the microphone",
        "camera" => "To use the camera",
        "autostart" => "To start when you log in",
        _ => "",
    }
    .to_string()
}

/// An optional dependency ticked at first: from what it is for (its
/// description), never from its name or the application's: sound, camera,
/// notifications, the tray, graphics.
pub fn optional_wanted(description: &str) -> bool {
    let d = description.to_lowercase();
    [
        "sound",
        "audio",
        "pulseaudio",
        "pipewire",
        "alsa",
        "microphone",
        "camera",
        "webcam",
        "notification",
        "tray",
        "indicator",
        "acceleration",
        "vaapi",
        "va-api",
        "vulkan",
        "opengl",
    ]
    .iter()
    .any(|k| d.contains(k))
}

/// The programs of Arch's `base` that its packages count on without
/// naming them (Arch's rule: a package does not depend on what `base` has):
/// the scripts that start an application call them (Steam's launcher
/// unpacks itself with tar). Only the user's programs: not systemd, pacman
/// or the kernel's tools.
pub const BASE_TOOLS: &[&str] = &[
    "bzip2",
    "coreutils",
    "file",
    "findutils",
    "gawk",
    "gettext",
    "grep",
    "gzip",
    "pciutils",
    "procps-ng",
    "psmisc",
    "sed",
    "tar",
    "util-linux",
    "xz",
];

/// The programs of `base` the application does not have yet.
pub fn missing_base(closure: &[String], packages: &[String]) -> Vec<String> {
    BASE_TOOLS
        .iter()
        .filter(|b| !closure.iter().chain(packages).any(|c| c == *b))
        .map(|b| b.to_string())
        .collect()
}

/// The package that sends ALSA to PipeWire, if the application needs it:
/// it has the permission `audio` and plays sound only through ALSA
/// (alsa-lib, but no PulseAudio or PipeWire library).
pub fn alsa_bridge(
    permissions: &[&str],
    closure: &[String],
    packages: &[String],
) -> Option<&'static str> {
    let has = |p: &str| closure.iter().chain(packages).any(|c| c == p);
    let other = ["libpulse", "pipewire", "pipewire-alsa", "pipewire-pulse"];
    (permissions.contains(&"audio") && has("alsa-lib") && !other.iter().any(|p| has(p)))
        .then_some("pipewire-alsa")
}

/// `--permission <id>[=<reason>]` (or `<catalog id>:<reason>`): the id
/// and the reason (empty: the default one).
pub fn parse_permission(arg: &str) -> Result<(String, String)> {
    let (id, reason) = match arg.split_once('=') {
        Some((id, r)) => (id.to_string(), r.to_string()),
        None => match arg.split_once(':') {
            Some((id, r)) if xapp_permissions::info(id).is_some() => {
                (id.to_string(), r.to_string())
            }
            _ => (arg.to_string(), String::new()),
        },
    };
    let id = id.trim().to_string();
    if !xapp_permissions::valid(&id) {
        return Err(err(format!(
            "unknown permission \"{id}\" (known: {})",
            xapp_permissions::known()
        )));
    }
    let reason = if reason.trim().is_empty() {
        default_reason(&id)
    } else {
        reason.trim().to_string()
    };
    if reason.is_empty() {
        return Err(err(format!(
            "the permission {id} needs a reason for the user: --permission '{id}=<reason>'"
        )));
    }
    Ok((id, reason))
}

/// One line of the permissions list.
struct Item {
    id: String,
    reason: String,
    ticked: bool,
    /// Why the wizard ticked it, shown to the author.
    because: String,
}

/// The permissions, like tasksel: every permission of the catalog, the
/// proposed ones ticked, then "Add a custom permission…" (dbus-talk,
/// dbus-own, folder, device), asked for until the author confirms.
/// `--yes`: what is ticked at first.
fn choose_permissions(proposals: &[Proposal], given: &[(String, String)]) -> Result<Vec<Value>> {
    let mut items: Vec<Item> = xapp_permissions::CATALOG
        .iter()
        .map(|i| {
            let proposal = proposals.iter().find(|p| p.id == i.id);
            let from_args = given.iter().find(|(id, _)| id == i.id);
            Item {
                id: i.id.to_string(),
                reason: from_args
                    .map(|(_, r)| r.clone())
                    .or_else(|| proposal.map(|p| p.reason.clone()))
                    .unwrap_or_else(|| default_reason(i.id)),
                ticked: proposal.is_some() || from_args.is_some(),
                because: match (from_args, proposal) {
                    (Some(_), _) => "--permission".into(),
                    (None, Some(p)) => p.because.clone(),
                    _ => String::new(),
                },
            }
        })
        .collect();
    for (id, reason) in given {
        if xapp_permissions::info(id).is_none() {
            items.push(Item {
                id: id.clone(),
                reason: reason.clone(),
                ticked: true,
                because: "--permission".into(),
            });
        }
    }
    loop {
        let mut list: Vec<(String, String, bool)> = items
            .iter()
            .map(|it| {
                let mut label = format!("{}: \"{}\"", xapp_permissions::label(&it.id), it.reason);
                if !it.because.is_empty() {
                    label.push_str(&format!(" ({})", it.because));
                }
                (it.id.clone(), label, it.ticked)
            })
            .collect();
        list.push(("custom".into(), "Add a custom permission…".into(), false));
        let q = Question::checklist(
            "xapp create: permissions",
            "What the application may ask the user for. The wizard ticked what it \
             probably needs; tick or untick with the space bar, then OK. The user \
             is asked about each one when the application first opens.",
            &list,
        );
        let answer = ui::ask(&[q]).ok_or_else(|| err("nobody to answer the wizard (use --yes)"))?;
        let ticked = ui::ticked(&answer[0]);
        for (n, it) in items.iter_mut().enumerate() {
            it.ticked = ticked.contains(&n);
        }
        if !ticked.contains(&items.len()) {
            break;
        }
        if let Some(it) = ask_custom()? {
            items.retain(|i| i.id != it.id);
            items.push(it);
        }
    }
    Ok(items
        .iter()
        .filter(|it| it.ticked)
        .map(|it| json!({"id": it.id, "reason": it.reason}))
        .collect())
}

/// "Add a custom permission…": its kind, its name, its reason.
fn ask_custom() -> Result<Option<Item>> {
    let kinds: Vec<String> = xapp_permissions::CUSTOM_KINDS
        .iter()
        .map(|(k, what)| format!("{k}: {what}"))
        .collect();
    let Some(kind) = ui::input(
        "xapp create: custom permission",
        &format!(
            "Kind of permission:\n{}\n\n(empty: back to the list)",
            kinds.join("\n")
        ),
        "",
    ) else {
        return Ok(None);
    };
    let kind = kind.trim().to_string();
    if kind.is_empty() {
        return Ok(None);
    }
    let example = match kind.as_str() {
        "dbus-talk" | "dbus-own" => "org.example.Service",
        "folder" => "~/Games  or  /mnt/data:ro",
        "device" => "input  or  all",
        _ => {
            println!("Not a kind of custom permission: {kind}");
            return Ok(None);
        }
    };
    let Some(value) = ui::input(
        "xapp create: custom permission",
        &format!("{kind}: which one? (for example {example})"),
        "",
    ) else {
        return Ok(None);
    };
    let id = format!("{kind}:{}", value.trim());
    let Some(c) = xapp_permissions::custom(&id) else {
        println!("Not a valid permission: {id}");
        return Ok(None);
    };
    let mut prompt = format!(
        "{}\n\nThe reason the user reads when asked (\"To read your saved games\"):",
        c.label()
    );
    if let Some(w) = c.warning() {
        prompt.push_str(&format!(
            "\n\nCareful: {w} The user is asked with \"no\" first."
        ));
    }
    let Some(reason) = ui::input("xapp create: custom permission", &prompt, "") else {
        return Ok(None);
    };
    if reason.trim().is_empty() {
        println!("A permission needs a reason: not added.");
        return Ok(None);
    }
    Ok(Some(Item {
        id: c.id(),
        reason: reason.trim().to_string(),
        ticked: true,
        because: "custom".into(),
    }))
}

/// The snapshot of this XeeonOS (XEEONOS_ARCH_SNAPSHOT in os-release).
fn system_snapshot() -> Option<String> {
    let text = fs::read_to_string("/usr/lib/os-release").ok()?;
    text.lines().find_map(|l| {
        let v = l.strip_prefix("XEEONOS_ARCH_SNAPSHOT=")?.trim_matches('"');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// pacman -Si: one field (first line of it).
fn si_field(info: &str, key: &str) -> String {
    info.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
        .unwrap_or_default()
}

/// The menu entry of the package: the one named like the package, else the
/// first one shown in the menu.
fn pick_desktop(dir: &Path, pkg: &str) -> Option<(String, Desktop)> {
    let mut entries: Vec<(String, Desktop)> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".desktop")?.to_string();
            let d = parse_desktop(&fs::read_to_string(e.path()).ok()?);
            (!d.hidden && !d.exec.is_empty()).then_some((stem, d))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let pkg = pkg.to_lowercase();
    let pos = entries
        .iter()
        .position(|(stem, _)| {
            let s = stem.to_lowercase();
            s == pkg || s.rsplit('.').next() == Some(pkg.as_str())
        })
        .unwrap_or(0);
    (!entries.is_empty()).then(|| entries.swap_remove(pos))
}

/// An application id for a package whose menu entry has none.
fn default_app_id(stem: &str, pkg: &str) -> String {
    if valid_app_id(stem) && stem.split('.').count() >= 3 {
        return stem.to_string();
    }
    let clean: String = pkg
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("org.xeeonos.arch.{clean}")
}

/// What the wizard found, before the questions.
struct Found {
    /// The default name of the recipe directory and of the work directory.
    short: String,
    app_id: String,
    name: String,
    names: Vec<(String, String)>,
    description: String,
    /// Shown in the summary; written in the recipe only for an archive.
    version: String,
    write_version: bool,
    exec: Vec<String>,
    icon: String,
    categories: Vec<String>,
    mime_types: Vec<String>,
    wm_class: String,
    packages: Vec<String>,
    /// Packages the program loads while running: asked before adding.
    loads: Vec<String>,
    closure: Vec<String>,
    /// Its own files (for the web engine) and what its programs name.
    files: Vec<String>,
    hints: Vec<String>,
    sources: Vec<Value>,
    /// `--aur`: {"name", "commit"}.
    aur: Vec<Value>,
    /// Why it needs the multilib repository (32-bit programs or packages):
    /// empty if it does not.
    multilib: Vec<String>,
    /// 32-bit graphics drivers to propose.
    gpu32: Vec<String>,
    /// The package's optional dependencies (name, what for), to propose.
    optdeps: Vec<(String, String)>,
    /// More lines for the summary.
    notes: Vec<String>,
}

/// How a script of the wizard runs.
#[derive(Default)]
struct Run<'a> {
    /// A directory of the user's, read-only on /source.
    source: Option<&'a Path>,
    /// Mounts inside (makepkg's root, --aur).
    privileged: bool,
}

/// Run a script of the wizard in an Arch Linux container, on work/.
fn in_container(
    work: &Path,
    script: &str,
    body: &str,
    args: &[&str],
    what: &str,
    how: Run,
) -> Result<()> {
    fs::write(work.join(script), body)?;
    fs::write(work.join("scan.sh"), SCAN_LIB)?;
    let engine = crate::engine()?;
    let mut cmd = Command::new(&engine);
    cmd.args(["run", "--rm"])
        .arg("--volume")
        .arg(format!("{}:/work", work.display()));
    if let Some(dir) = how.source {
        cmd.arg("--volume")
            .arg(format!("{}:/source:ro", dir.display()));
    }
    if how.privileged {
        cmd.arg("--privileged");
    }
    if engine.ends_with("docker") {
        cmd.args(["--env", &format!("XAPP_CHOWN={}:{}", uid(), gid())]);
    }
    cmd.arg(crate::container_image())
        .args(["/bin/bash", &format!("/work/{script}")])
        .args(args);
    println!("{what} ({engine})...");
    let status = cmd
        .status()
        .map_err(|e| err(format!("cannot run {engine}: {e}")))?;
    if !status.success() {
        return Err(err(format!("{what}: failed ({status})")));
    }
    Ok(())
}

/// The lines of a file of work/ (none if it is missing).
fn lines_of(work: &Path, f: &str) -> Vec<String> {
    fs::read_to_string(work.join(f))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// The packages of a "library package" file of work/ (glibc left out),
/// sorted, once each.
fn lib_packages(work: &Path, f: &str) -> Vec<String> {
    let mut v: Vec<String> = lines_of(work, f)
        .iter()
        .filter_map(|l| l.split_once(' ').map(|(_, p)| p.to_string()))
        .filter(|p| p != "glibc")
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The lines of a file of work/ that are not empty.
fn non_empty_lines(work: &Path, f: &str) -> Vec<String> {
    lines_of(work, f)
        .into_iter()
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// The application id given with --id, checked.
fn chosen_id(o: &Options) -> Result<Option<String>> {
    match &o.app_id {
        Some(id) if valid_app_id(id) => Ok(Some(id.clone())),
        Some(id) => Err(err(format!("\"{id}\" is not a valid application id"))),
        None => Ok(None),
    }
}

/// `--arch <package>`.
fn from_arch(o: &Options, pkg: &str, snapshot: &str, work: &Path) -> Result<Found> {
    in_container(
        work,
        "create-arch.sh",
        ARCH_SCRIPT,
        &[snapshot, pkg],
        &format!("Looking at {pkg} in the Arch snapshot {snapshot}"),
        Run::default(),
    )?;
    from_package(o, pkg, work)
}

/// The packages that are not in the snapshot.
fn not_in_snapshot(work: &Path) -> Vec<String> {
    non_empty_lines(work, "missing.txt")
}

/// The question before building a PKGBUILD: "Don't build" first, so that
/// `--yes` never builds code nobody read.
fn pkgbuild_question(pkg: &str, commit: &str, pkgbuild: &str) -> Question {
    let mut q = Question::yes_no(
        &format!("xapp create: the PKGBUILD of {pkg}"),
        &format!(
            "The AUR package {pkg} (commit {commit}) is built from this PKGBUILD, \
             written by its maintainer, not checked by Arch Linux. It runs in a \
             container, as a normal user, and the application gets only what it \
             installs. Build it?\n\n{pkgbuild}"
        ),
        "Build it",
        "Don't build",
        false,
    );
    q.choices.reverse();
    q.default = 0;
    q
}

/// `--aur <package>`: fetch, show the PKGBUILD, build, then as a package.
fn from_aur(o: &Options, pkg: &str, snapshot: &str, work: &Path) -> Result<Found> {
    if !crate::valid_aur_name(pkg) {
        return Err(err(format!("\"{pkg}\" is not an AUR package name")));
    }
    fs::write(work.join("aur.sh"), crate::AUR_LIB)?;
    fs::write(work.join("create-arch.sh"), ARCH_SCRIPT)?;
    in_container(
        work,
        "create-aur.sh",
        AUR_SCRIPT,
        &[snapshot, "fetch", pkg],
        &format!("Fetching {pkg} from the AUR"),
        Run::default(),
    )?;
    let commit = lines_of(work, "commit.txt")
        .into_iter()
        .next()
        .unwrap_or_default();
    if commit.len() != 40 {
        return Err(err(format!("no commit for the AUR package {pkg}")));
    }
    let missing = not_in_snapshot(work);
    if !missing.is_empty() {
        return Err(err(format!(
            "{pkg} needs packages that the Arch snapshot {snapshot} does not have \
             (other AUR packages, not built by xapp yet, or newer ones: --snapshot): {}",
            missing.join(" ")
        )));
    }
    let pkgbuild_path = work.join("aur/PKGBUILD");
    let pkgbuild = fs::read_to_string(&pkgbuild_path)?;
    if !o.pkgbuild_ok {
        // On the terminal too: a window shows only part of a long one.
        println!(
            "--- PKGBUILD of {pkg} ({}) ---\n{pkgbuild}\n---",
            pkgbuild_path.display()
        );
        let answer = ui::ask(&[pkgbuild_question(pkg, &commit, &pkgbuild)])
            .ok_or_else(|| err("nobody to read the PKGBUILD"))?;
        if answer.first().is_none_or(|a| a != "yes") {
            return Err(err(format!(
                "the PKGBUILD of {pkg} was not built (it is in {}; --pkgbuild-ok builds it without asking)",
                pkgbuild_path.display()
            )));
        }
    }
    in_container(
        work,
        "create-aur.sh",
        AUR_SCRIPT,
        &[snapshot, "build", pkg],
        &format!("Building {pkg} (AUR) against the Arch snapshot {snapshot}"),
        Run {
            source: None,
            privileged: true,
        },
    )?;
    let mut f = from_package(o, pkg, work)?;
    // Not a package of the snapshot: built from the AUR by xapp build.
    f.packages.retain(|p| p != pkg);
    f.aur = vec![json!({"name": pkg, "commit": commit})];
    f.notes = vec![
        format!("From the AUR: {pkg}, commit {commit}"),
        format!("Packages: {} (from {pkg})", f.closure.len()),
    ];
    Ok(f)
}

/// A package looked at by create-arch.sh (from the snapshot or the AUR).
fn from_package(o: &Options, pkg: &str, work: &Path) -> Result<Found> {
    let info = fs::read_to_string(work.join("info.txt"))?;
    let closure = lines_of(work, "closure.txt");
    let files = lines_of(work, "files.txt");
    let (stem, d) = pick_desktop(&work.join("pkgroot/usr/share/applications"), pkg).ok_or_else(|| {
        err(format!(
            "{pkg} has no menu entry (.desktop): it is not an application with windows, or its entry is in another package"
        ))
    })?;
    let app_id = chosen_id(o)?.unwrap_or_else(|| default_app_id(&stem, pkg));
    let exec = command(&d.exec, &files)
        .ok_or_else(|| err(format!("the menu entry of {pkg} has no command")))?;
    // The icon: in the package, else in an icon theme (then that package
    // goes in the recipe too, unless the package already brings it).
    let mut packages = vec![pkg.to_string()];
    let icon = match icon_path(&d.icon, &files) {
        Some(icon) => icon,
        None => {
            let (owner, icon) = theme_icon(&d.icon, &lines_of(work, "icons.txt"), &closure)
                .ok_or_else(|| {
                    err(format!(
                        "no icon \"{}\" in {pkg} or in an icon theme: write the recipe's icon by hand",
                        d.icon
                    ))
                })?;
            if !closure.contains(&owner) {
                packages.push(owner);
            }
            icon
        }
    };
    let description = if d.comment.is_empty() {
        si_field(&info, "Description")
    } else {
        d.comment.clone()
    };
    let wm_class = if d.wm_class.is_empty() {
        stem.clone()
    } else {
        d.wm_class.clone()
    };
    Ok(Found {
        short: pkg.to_string(),
        app_id,
        name: d.name,
        names: d.names,
        description,
        version: si_field(&info, "Version"),
        write_version: false,
        exec,
        icon,
        categories: d.categories,
        mime_types: d.mime_types,
        wm_class,
        notes: vec![format!("Packages: {} (from {pkg})", closure.len())],
        loads: lib_packages(work, "loads.txt")
            .into_iter()
            .filter(|p| !packages.contains(p) && !closure.contains(p))
            .collect(),
        packages,
        closure,
        files,
        hints: lines_of(work, "hints.txt"),
        sources: Vec::new(),
        aur: Vec::new(),
        multilib: non_empty_lines(work, "multilib.txt"),
        gpu32: non_empty_lines(work, "gpu32.txt"),
        optdeps: non_empty_lines(work, "optdeps.txt")
            .iter()
            .filter_map(|l| {
                let (n, d) = l.split_once('\t')?;
                Some((n.to_string(), d.to_string()))
            })
            .collect(),
    })
}

/// The file name of an address, without the query.
fn url_file(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/').next().unwrap_or_default().to_string()
}

/// "firefox-147.0.1.tar.xz" -> ("firefox", "147.0.1"): the name, and the
/// version if the file name has one.
pub fn name_and_version(file: &str) -> (String, String) {
    let mut stem = file.to_string();
    for ext in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".zip", ".tar",
    ] {
        if let Some(s) = stem.strip_suffix(ext) {
            stem = s.to_string();
            break;
        }
    }
    // The version starts at the first "-", "_" or "." followed by a digit.
    let b = stem.as_bytes();
    let cut = (1..b.len()).find(|&i| {
        matches!(b[i - 1], b'-' | b'_' | b'.' | b'v') && b[i].is_ascii_digit() && {
            // "v1.2": the v belongs to the version.
            b[i - 1] != b'v' || (i >= 2 && matches!(b[i - 2], b'-' | b'_'))
        }
    });
    let (name, version) = match cut {
        Some(i) => {
            let start = if b[i - 1] == b'v' { i - 2 } else { i - 1 };
            (&stem[..start], &stem[i..])
        }
        None => (stem.as_str(), ""),
    };
    // The version ends where the platform begins (linux-x86_64, amd64...).
    let version = version
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    (name.to_lowercase(), version.to_string())
}

/// An application id from where the application comes from:
/// download.mozilla.org + firefox -> org.mozilla.firefox,
/// github.com/owner/... -> io.github.owner.<name>.
pub fn id_from_url(url: &str, name: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    };
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or_default();
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let id = if host == "github.com" || host == "gitlab.com" || host == "codeberg.org" {
        let owner = parts.next().unwrap_or("app");
        let tld = if host == "github.com" {
            "io.github"
        } else if host == "gitlab.com" {
            "io.gitlab"
        } else {
            "org.codeberg"
        };
        format!("{tld}.{}.{}", clean(owner), clean(name))
    } else if labels.len() >= 2 && !labels.iter().all(|l| l.chars().all(|c| c.is_ascii_digit())) {
        let n = labels.len();
        format!(
            "{}.{}.{}",
            clean(labels[n - 1]),
            clean(labels[n - 2]),
            clean(name)
        )
    } else {
        format!("org.xeeonos.app.{}", clean(name))
    };
    if valid_app_id(&id) {
        id
    } else {
        format!("org.xeeonos.app.{}", clean(name))
    }
}

/// The program to start, among the archive's programs: `--exec`, else the
/// one named like the application at the top, else the only one at the top.
pub fn pick_program(programs: &[String], name: &str, chosen: Option<&str>) -> Option<String> {
    if let Some(c) = chosen {
        return programs
            .iter()
            .find(|p| p.as_str() == c.trim_start_matches('/'))
            .cloned();
    }
    let top: Vec<&String> = programs.iter().filter(|p| !p.contains('/')).collect();
    let named = |list: &[&String]| {
        list.iter()
            .find(|p| {
                p.eq_ignore_ascii_case(name) || p.to_lowercase().starts_with(&format!("{name}-bin"))
            })
            .map(|p| p.to_string())
    };
    named(&top)
        .or_else(|| (top.len() == 1).then(|| top[0].clone()))
        .or_else(|| {
            let all: Vec<&String> = programs.iter().collect();
            named(&all)
        })
}

/// The icon of an archive: an image named after the menu entry's Icon=,
/// else the biggest image in a directory or file called *icon* (SVG first).
pub fn archive_icon(files: &[String], icon_name: &str) -> Option<String> {
    let size = |path: &str| -> u32 {
        let name = path.rsplit('/').next().unwrap_or_default();
        let digits = |s: &str| -> u32 {
            s.split(|c: char| !c.is_ascii_digit())
                .filter_map(|d| d.parse::<u32>().ok())
                .filter(|n| *n <= 1024)
                .max()
                .unwrap_or(0)
        };
        digits(name).max(path.split('/').map(digits).max().unwrap_or(0))
    };
    let images = files
        .iter()
        .filter(|f| f.ends_with(".png") || f.ends_with(".svg"));
    let best = |list: Vec<&String>| {
        list.into_iter()
            .max_by_key(|f| (f.ends_with(".svg"), size(f), std::cmp::Reverse(f.len())))
            .cloned()
    };
    if !icon_name.is_empty() {
        let named: Vec<&String> = images
            .clone()
            .filter(|f| {
                let n = f.rsplit('/').next().unwrap_or_default();
                n.strip_suffix(".png").or_else(|| n.strip_suffix(".svg")) == Some(icon_name)
            })
            .collect();
        if let Some(f) = best(named) {
            return Some(f);
        }
    }
    best(
        images
            .filter(|f| f.to_lowercase().contains("icon") && !f.contains("symbolic"))
            .collect(),
    )
}

/// `--url <address>`: an archive with the application's own files, put in
/// /opt/<name> of the image; the libraries it needs come from packages.
/// `--dir <directory>`: the same, from a directory of the user's.
fn from_files(o: &Options, origin: &Source, snapshot: &str, work: &Path) -> Result<Found> {
    let (arg, shown, run) = match origin {
        Source::Url(url) => (url.clone(), url.clone(), Run::default()),
        Source::Dir(dir) => (
            "/source".to_string(),
            dir.display().to_string(),
            Run {
                source: Some(dir),
                privileged: false,
            },
        ),
        _ => return Err(err("not an archive or a directory")),
    };
    in_container(
        work,
        "create-url.sh",
        URL_SCRIPT,
        &[snapshot, &arg],
        &format!("Looking at {shown} (libraries from the Arch snapshot {snapshot})"),
        run,
    )?;
    let first = |f: &str| lines_of(work, f).into_iter().next().unwrap_or_default();
    let (final_url, sha256, from_name) = match origin {
        Source::Dir(dir) => (
            String::new(),
            String::new(),
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        _ => {
            let final_url = first("url.txt");
            let sha256 = first("sha256.txt");
            if sha256.len() != 64 {
                return Err(err("the download has no SHA-256"));
            }
            let file = url_file(&final_url);
            (final_url, sha256, file)
        }
    };
    let files = lines_of(work, "files.txt");
    let programs = lines_of(work, "programs.txt");
    let (file_name, file_version) = name_and_version(&from_name);
    let top = first("top.txt");
    let short = if !top.is_empty() && name_and_version(&top).0.len() > 1 {
        name_and_version(&top).0
    } else {
        file_name.clone()
    };
    let short: String = short
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if short.is_empty() {
        return Err(err(format!(
            "cannot tell the application's name from {shown} (--id, and a directory with a name)"
        )));
    }
    let to = format!("/opt/{short}");

    // A menu entry inside the archive, if there is one.
    let desktop = files
        .iter()
        .filter(|f| f.ends_with(".desktop"))
        .filter_map(|f| {
            let d = parse_desktop(&fs::read_to_string(work.join("tree").join(f)).ok()?);
            (!d.hidden).then_some(d)
        })
        .next()
        .unwrap_or_default();
    let from_desktop = exec_words(&desktop.exec)
        .first()
        .map(|w| w.rsplit('/').next().unwrap_or_default().to_string());
    let program = o
        .exec
        .as_deref()
        .and_then(|e| pick_program(&programs, &short, Some(e)))
        .or_else(|| {
            from_desktop.as_ref().and_then(|n| {
                programs
                    .iter()
                    .find(|p| p.rsplit('/').next() == Some(n.as_str()))
                    .cloned()
            })
        })
        .or_else(|| pick_program(&programs, &short, None))
        .ok_or_else(|| {
            err(format!(
                "which program starts it? Programs in it: {} (--exec <one of them>)",
                if programs.is_empty() {
                    "none".to_string()
                } else {
                    programs.join(", ")
                }
            ))
        })?;
    let mut exec = vec![format!("{to}/{program}")];
    exec.extend(exec_words(&desktop.exec).into_iter().skip(1));
    let icon = match &o.icon {
        Some(i) => i.trim_start_matches('/').to_string(),
        None => archive_icon(&files, &desktop.icon)
            .ok_or_else(|| err("no icon in it (--icon <path of a .png or .svg in it>)"))?,
    };
    if !files.contains(&icon) {
        return Err(err(format!("no {icon} in {shown}")));
    }

    let packages = lib_packages(work, "needs.txt");
    let loads: Vec<String> = lib_packages(work, "loads.txt")
        .into_iter()
        .filter(|p| !packages.contains(p))
        .collect();
    let missing = lines_of(work, "missing.txt");
    let mut notes = match origin {
        Source::Dir(_) => vec![format!("From the directory: {shown}")],
        _ => vec![format!("From: {final_url}"), format!("SHA-256: {sha256}")],
    };
    notes.extend([
        format!("Into: {to}"),
        format!(
            "Libraries from packages: {}",
            if packages.is_empty() {
                "none".into()
            } else {
                packages.join(" ")
            }
        ),
    ]);
    if !missing.is_empty() {
        notes.push(format!(
            "Not in any package (it may not start): {}",
            missing.join(" ")
        ));
    }
    let name = if desktop.name.is_empty() {
        let mut c = short.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    } else {
        desktop.name.clone()
    };
    let version = if file_version.is_empty() {
        name_and_version(&top).1
    } else {
        file_version
    };
    let version = if version.is_empty() {
        "1".to_string()
    } else {
        version
    };
    Ok(Found {
        app_id: chosen_id(o)?.unwrap_or_else(|| match origin {
            Source::Url(url) => id_from_url(url, &short),
            _ => id_from_url("", &short),
        }),
        description: if desktop.comment.is_empty() {
            name.clone()
        } else {
            desktop.comment.clone()
        },
        names: desktop.names,
        name,
        version,
        write_version: true,
        exec,
        icon: format!("{to}/{icon}"),
        categories: desktop.categories,
        mime_types: desktop.mime_types,
        wm_class: if desktop.wm_class.is_empty() {
            short.clone()
        } else {
            desktop.wm_class
        },
        packages,
        loads,
        closure: lines_of(work, "closure.txt"),
        files,
        hints: lines_of(work, "hints.txt"),
        sources: vec![match origin {
            Source::Dir(dir) => json!({"dir": dir.display().to_string(), "to": to}),
            _ => json!({"url": final_url, "sha256": sha256, "to": to}),
        }],
        aur: Vec::new(),
        multilib: non_empty_lines(work, "multilib.txt"),
        gpu32: non_empty_lines(work, "gpu32.txt"),
        optdeps: Vec::new(),
        notes,
        short,
    })
}

/// Make the recipe; returns its directory.
pub fn create(o: &Options) -> Result<PathBuf> {
    let snapshot = o
        .snapshot
        .clone()
        .or_else(system_snapshot)
        .ok_or_else(|| err("which Arch snapshot? (--snapshot YYYY/MM/DD)"))?;
    if xapp_core::date::parse(&snapshot).is_none() {
        return Err(err("the snapshot must be a date, YYYY/MM/DD"));
    }
    // A directory: as an absolute path (the recipe is elsewhere).
    let dir_source;
    let source = match &o.source {
        Source::Dir(d) => {
            let d = fs::canonicalize(d)
                .ok()
                .filter(|d| d.is_dir())
                .ok_or_else(|| err(format!("{} is not a directory", d.display())))?;
            if d.to_string_lossy().contains([':', ',']) {
                return Err(err(format!(
                    "{}: a directory with \":\" or \",\" in its path cannot be given to the container",
                    d.display()
                )));
            }
            dir_source = Source::Dir(d);
            &dir_source
        }
        s => s,
    };
    let key = match source {
        Source::Arch(pkg) | Source::Aur(pkg) => pkg.clone(),
        Source::Dir(d) => d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "dir".into()),
        Source::Url(url) => {
            if !url.starts_with("https://") {
                return Err(err("the address must start with https://"));
            }
            let (name, _) = name_and_version(&url_file(url));
            if name.is_empty() {
                "download".into()
            } else {
                name
            }
        }
    };
    let work = o
        .work
        .clone()
        .unwrap_or_else(|| paths::cache_home().join("xapp/create").join(&key));
    fs::create_dir_all(&work)?;
    let work = fs::canonicalize(&work)?;
    if let Some(out) = &o.output
        && out.join("recipe.json").exists()
    {
        return Err(err(format!("{} already has a recipe.json", out.display())));
    }
    let f = match source {
        Source::Arch(pkg) => from_arch(o, pkg, &snapshot, &work)?,
        Source::Aur(pkg) => from_aur(o, pkg, &snapshot, &work)?,
        Source::Url(_) | Source::Dir(_) => from_files(o, source, &snapshot, &work)?,
    };
    let output = o.output.clone().unwrap_or_else(|| PathBuf::from(&f.short));
    if output.join("recipe.json").exists() {
        return Err(err(format!(
            "{} already has a recipe.json",
            output.display()
        )));
    }

    // The questions: the summary, the libraries it loads (if any), then
    // the permissions, one by one.
    let given: Vec<(String, String)> = o
        .permissions
        .iter()
        .map(|p| parse_permission(p))
        .collect::<Result<_>>()?;
    let proposals = propose(&f.closure, &f.categories, &f.mime_types, &f.files, &f.hints);
    let mut notes = f.notes.clone();
    if !f.multilib.is_empty() {
        notes.push(format!(
            "Needs the multilib repository (32-bit): {}",
            f.multilib.join("; ")
        ));
    }
    let summary = format!(
        "{} {} ({})\n{}\n\nCommand: {}\nIcon: {}\n{}\n\n\
         Next: the list of permissions, with those it probably needs ticked.",
        f.name,
        f.version,
        f.app_id,
        f.description,
        f.exec.join(" "),
        f.icon,
        notes.join("\n"),
    );
    let mut questions = vec![Question::message("xapp create", &summary)];
    if !f.loads.is_empty() {
        questions.push(Question::yes_no(
            "xapp create: libraries",
            &format!(
                "Its programs name libraries they may load while running (sound, video, \
                 graphics, printing...). Put their packages in the application too?\n\n{}",
                f.loads.join(" ")
            ),
            "Put them in",
            "Leave them out",
            true,
        ));
    }
    if !f.gpu32.is_empty() {
        questions.push(Question::yes_no(
            "xapp create: 32-bit graphics",
            &format!(
                "It has 32-bit programs that draw with OpenGL or Vulkan, and XeeonOS has \
                 no 32-bit graphics drivers to lend them. Put these in the application?\n\n{}",
                f.gpu32.join(" ")
            ),
            "Put them in",
            "Leave them out",
            true,
        ));
    }
    if !f.optdeps.is_empty() {
        let items: Vec<(String, String, bool)> = f
            .optdeps
            .iter()
            .map(|(n, d)| (n.clone(), format!("{n}: {d}"), optional_wanted(d)))
            .collect();
        let mut q = Question::checklist(
            "xapp create: optional packages",
            "Its packagers list these as optional: what it can use if they are there \
             (some it may download or load only when it runs). Tick what to put in the \
             application; ticked: sound, camera, notifications, tray, graphics.",
            &items,
        );
        let ticked: Vec<usize> = (0..items.len()).filter(|&i| items[i].2).collect();
        q.yes = Some(Question::ticks_answer(&ticked));
        questions.push(q);
    }
    let base = missing_base(&f.closure, &f.packages);
    if !base.is_empty() {
        questions.push(Question::yes_no(
            "xapp create: basic programs",
            &format!(
                "Arch packages count on the programs of a whole system (tar, sed, grep...) \
                 without naming them, and the scripts that start applications call them. \
                 Put those it does not have in the application?\n\n{}",
                base.join(" ")
            ),
            "Put them in",
            "Leave them out",
            true,
        ));
    }
    let answers =
        ui::ask(&questions).ok_or_else(|| err("nobody to answer the wizard (use --yes)"))?;
    // The answers after the summary, in the order of the questions.
    let mut rest = answers.iter().skip(1);
    let mut packages = f.packages.clone();
    if !f.loads.is_empty() && rest.next().is_some_and(|a| a == "yes") {
        packages.extend(f.loads.iter().cloned());
    }
    let mut multilib = !f.multilib.is_empty();
    if !f.gpu32.is_empty() && rest.next().is_some_and(|a| a == "yes") {
        packages.extend(f.gpu32.iter().cloned());
        multilib = true;
    }
    if !f.optdeps.is_empty()
        && let Some(a) = rest.next()
    {
        for i in ui::ticked(a) {
            if let Some((n, _)) = f.optdeps.get(i)
                && !packages.contains(n)
            {
                packages.push(n.clone());
            }
        }
    }
    if !base.is_empty() && rest.next().is_some_and(|a| a == "yes") {
        packages.extend(
            base.iter()
                .filter(|b| !packages.contains(b))
                .cloned()
                .collect::<Vec<_>>(),
        );
    }
    let permissions = choose_permissions(&proposals, &given)?;
    // Sound for an application that speaks only ALSA: through PipeWire.
    let ids: Vec<&str> = permissions
        .iter()
        .filter_map(|p| p.get("id").and_then(Value::as_str))
        .collect();
    if let Some(bridge) = alsa_bridge(&ids, &f.closure, &packages) {
        let q = Question::yes_no(
            "xapp create: sound",
            &format!(
                "It plays sound only through ALSA (alsa-lib, without PulseAudio or PipeWire \
                 libraries). In the sandbox there is no sound card, only PipeWire: put {bridge} \
                 in the application, so that ALSA goes to PipeWire?"
            ),
            "Put it in",
            "Leave it out",
            true,
        );
        let a = ui::ask(&[q]).ok_or_else(|| err("nobody to answer the wizard (use --yes)"))?;
        if a.first().is_some_and(|a| a == "yes") {
            packages.push(bridge.to_string());
        }
    }

    let names: Map<String, Value> = f
        .names
        .iter()
        .filter(|(_, n)| *n != f.name)
        .map(|(l, n)| (l.clone(), json!(n)))
        .collect();
    let mut recipe = Map::new();
    recipe.insert("app_id".into(), json!(f.app_id));
    recipe.insert("name".into(), json!(f.name));
    if !names.is_empty() {
        recipe.insert("names".into(), Value::Object(names));
    }
    recipe.insert("description".into(), json!(f.description));
    if f.write_version {
        recipe.insert("version".into(), json!(f.version));
    }
    recipe.insert("snapshot".into(), json!(snapshot));
    recipe.insert("packages".into(), json!(packages));
    if multilib {
        recipe.insert("multilib".into(), json!(true));
    }
    if !f.sources.is_empty() {
        recipe.insert("sources".into(), Value::Array(f.sources.clone()));
    }
    if !f.aur.is_empty() {
        recipe.insert("aur".into(), Value::Array(f.aur.clone()));
    }
    recipe.insert("exec".into(), json!(f.exec));
    recipe.insert("gpu".into(), json!("host"));
    recipe.insert("permissions".into(), Value::Array(permissions));
    if !f.mime_types.is_empty() {
        recipe.insert("mime_types".into(), json!(f.mime_types));
    }
    if !f.categories.is_empty() {
        recipe.insert("categories".into(), json!(f.categories));
    }
    recipe.insert("wm_class".into(), json!(f.wm_class));
    recipe.insert("icon".into(), json!(f.icon));
    let mut text = serde_json::to_vec_pretty(&Value::Object(recipe)).unwrap_or_default();
    text.push(b'\n');
    fs::create_dir_all(&output)?;
    fs::write(output.join("recipe.json"), text)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = "[Desktop Entry]\nName=Calculator\nName[it]=Calcolatrice\nName[sr@latin]=Kalk\n\
        Comment=Perform calculations\nExec=gnome-calculator %U\nIcon=org.gnome.Calculator\n\
        Categories=GNOME;GTK;Utility;Calculator;\nMimeType=text/plain;\nStartupNotify=true\n\
        [Desktop Action new]\nName=New\nExec=other\n";

    #[test]
    fn desktop_files() {
        let d = parse_desktop(DESKTOP);
        assert_eq!(d.name, "Calculator");
        assert_eq!(
            d.names,
            vec![("it".to_string(), "Calcolatrice".to_string())]
        );
        assert_eq!(d.exec, "gnome-calculator %U");
        assert_eq!(d.categories, ["GNOME", "GTK", "Utility", "Calculator"]);
        assert!(!d.hidden);
        assert!(parse_desktop("[Desktop Entry]\nNoDisplay=true\n").hidden);
    }

    #[test]
    fn commands() {
        let files = vec![
            "usr/bin/gnome-calculator".to_string(),
            "usr/lib/foo/foo".to_string(),
        ];
        assert_eq!(
            command("gnome-calculator %U", &files).unwrap(),
            ["/usr/bin/gnome-calculator"]
        );
        assert_eq!(
            command("foo --new-window %u", &files).unwrap(),
            ["/usr/lib/foo/foo", "--new-window"]
        );
        assert_eq!(
            command("env GDK_BACKEND=x11 \"/opt/My App/app\" --x", &files).unwrap(),
            ["/opt/My App/app", "--x"]
        );
        assert!(command("", &files).is_none());
    }

    #[test]
    fn icons() {
        let files: Vec<String> = [
            "usr/share/icons/hicolor/48x48/apps/a.png",
            "usr/share/icons/hicolor/256x256/apps/a.png",
            "usr/share/icons/hicolor/scalable/apps/b.svg",
            "usr/share/icons/hicolor/256x256/apps/b.png",
            "usr/share/pixmaps/c.png",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            icon_path("a", &files).unwrap(),
            "/usr/share/icons/hicolor/256x256/apps/a.png"
        );
        assert_eq!(
            icon_path("b", &files).unwrap(),
            "/usr/share/icons/hicolor/scalable/apps/b.svg"
        );
        assert_eq!(icon_path("c", &files).unwrap(), "/usr/share/pixmaps/c.png");
        assert!(icon_path("d", &files).is_none());
    }

    #[test]
    fn theme_icons() {
        let candidates: Vec<String> = [
            "breeze-icons usr/share/icons/breeze/apps/48/accessories-calculator.svg",
            "breeze-icons usr/share/icons/breeze/apps/16/accessories-calculator.svg",
            "adwaita-icon-theme usr/share/icons/Adwaita/symbolic/apps/accessories-calculator-symbolic.svg",
            "oxygen-icons usr/share/icons/oxygen/base/256x256/apps/accessories-calculator.png",
            "hicolor-x usr/share/icons/hicolor/48x48/apps/accessories-calculator-old.png",
        ]
        .map(String::from)
        .to_vec();
        let in_closure = vec!["breeze-icons".to_string()];
        assert_eq!(
            theme_icon("accessories-calculator", &candidates, &in_closure).unwrap(),
            (
                "breeze-icons".into(),
                "/usr/share/icons/breeze/apps/48/accessories-calculator.svg".into()
            )
        );
        assert_eq!(
            theme_icon("accessories-calculator", &candidates, &[])
                .unwrap()
                .0,
            "breeze-icons"
        );
        assert!(theme_icon("nothing", &candidates, &[]).is_none());
    }

    #[test]
    fn proposals() {
        let closure: Vec<String> = ["glibc", "gtk4", "libadwaita", "pipewire", "webkitgtk-6.0"]
            .map(String::from)
            .to_vec();
        let ids: Vec<&str> = propose(
            &closure,
            &["Utility".into()],
            &["image/png".into()],
            &[],
            &[],
        )
        .iter()
        .map(|p| p.id)
        .collect();
        assert_eq!(
            ids,
            [
                "network",
                "gpu",
                "notifications",
                "audio",
                "pictures",
                "camera"
            ]
        );
        assert!(propose(&["glibc".into()], &[], &[], &[], &[]).is_empty());
        // Every proposal is a known permission, with a reason.
        for p in propose(
            &closure,
            &["WebBrowser".into()],
            &["text/html".into(), "video/mp4".into()],
            &[],
            &[],
        ) {
            assert!(xapp_permissions::info(p.id).is_some(), "{}", p.id);
            assert!(!p.reason.is_empty());
        }
        // Electron inside (Discord): calls, whatever the packages say.
        let electron = ["chrome-sandbox".to_string(), "Discord".into()];
        let ids: Vec<&str> = propose(&["glibc".into()], &[], &[], &electron, &[])
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["audio", "camera", "notifications"]);
        // A chat by its category; a camera named in a program.
        let ids: Vec<&str> = propose(&[], &["InstantMessaging".into()], &[], &[], &[])
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["audio", "camera", "notifications"]);
        let ids: Vec<&str> = propose(&[], &[], &[], &[], &["camera /dev/video".into()])
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["camera"]);
    }

    #[test]
    fn optional_dependencies() {
        assert!(optional_wanted("Pulseaudio support"));
        assert!(optional_wanted("Systray indicator support"));
        assert!(optional_wanted("Hardware video acceleration"));
        assert!(!optional_wanted("Open files"));
        assert!(!optional_wanted("Spell checking"));
    }

    #[test]
    fn sound_through_pipewire() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let alsa_only = v(&["glibc", "alsa-lib"]);
        assert_eq!(
            alsa_bridge(&["audio"], &alsa_only, &[]),
            Some("pipewire-alsa")
        );
        // Not without the permission, nor with a PulseAudio library (the
        // closure's or one the user put in).
        assert_eq!(alsa_bridge(&["network"], &alsa_only, &[]), None);
        assert_eq!(alsa_bridge(&["audio"], &alsa_only, &v(&["libpulse"])), None);
        assert_eq!(
            alsa_bridge(&["audio"], &v(&["alsa-lib", "pipewire"]), &[]),
            None
        );
        assert_eq!(alsa_bridge(&["audio"], &v(&["glibc"]), &[]), None);
    }

    #[test]
    fn permissions_from_the_command_line() {
        assert_eq!(
            parse_permission("camera").unwrap(),
            ("camera".into(), "To use the camera".into())
        );
        assert_eq!(
            parse_permission("camera:Per le videochiamate").unwrap(),
            ("camera".into(), "Per le videochiamate".into())
        );
        assert_eq!(
            parse_permission("folder:~/Giochi:ro=Per i salvataggi").unwrap(),
            ("folder:~/Giochi:ro".into(), "Per i salvataggi".into())
        );
        // A custom permission needs its reason.
        assert!(parse_permission("device:input").is_err());
        assert!(parse_permission("everything").is_err());
        for i in xapp_permissions::CATALOG {
            assert!(!default_reason(i.id).is_empty(), "{}", i.id);
        }
    }

    #[test]
    fn app_ids() {
        assert_eq!(
            default_app_id("org.gnome.Calculator", "gnome-calculator"),
            "org.gnome.Calculator"
        );
        assert_eq!(default_app_id("gimp", "gimp"), "org.xeeonos.arch.gimp");
        assert_eq!(default_app_id("vlc", "vlc-git"), "org.xeeonos.arch.vlc_git");
    }

    #[test]
    fn names_and_versions() {
        let nv = |f: &str| {
            let (n, v) = name_and_version(f);
            (n, v)
        };
        assert_eq!(
            nv("firefox-147.0.1.tar.xz"),
            ("firefox".into(), "147.0.1".into())
        );
        assert_eq!(
            nv("Obsidian-1.8.10.tar.gz"),
            ("obsidian".into(), "1.8.10".into())
        );
        assert_eq!(
            nv("app_v2.3-linux-x86_64.tar.gz"),
            ("app".into(), "2.3".into())
        );
        assert_eq!(nv("thunderbird.tar.bz2"), ("thunderbird".into(), "".into()));
        assert_eq!(nv("libreoffice7.zip").0, "libreoffice7");
    }

    #[test]
    fn ids_from_urls() {
        assert_eq!(
            id_from_url(
                "https://download-installer.cdn.mozilla.net/pub/firefox/x.tar.xz",
                "firefox"
            ),
            "net.mozilla.firefox"
        );
        assert_eq!(
            id_from_url(
                "https://ftp.mozilla.org/pub/firefox/releases/147.0/linux-x86_64/en-US/firefox-147.0.tar.xz",
                "firefox"
            ),
            "org.mozilla.firefox"
        );
        assert_eq!(
            id_from_url(
                "https://github.com/Some-Owner/tool/releases/download/v1/tool.tar.gz",
                "tool"
            ),
            "io.github.some_owner.tool"
        );
        assert_eq!(
            id_from_url("https://10.0.0.1/a.tar", "a"),
            "org.xeeonos.app.a"
        );
    }

    #[test]
    fn programs() {
        let p: Vec<String> = [
            "firefox",
            "firefox-bin",
            "glxtest",
            "plugin-container",
            "gmp/helper",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(pick_program(&p, "firefox", None).unwrap(), "firefox");
        assert_eq!(
            pick_program(&p, "firefox", Some("/glxtest")).unwrap(),
            "glxtest"
        );
        assert!(pick_program(&p, "firefox", Some("nothing")).is_none());
        let one = vec!["App".to_string(), "lib/helper".to_string()];
        assert_eq!(pick_program(&one, "other", None).unwrap(), "App");
        let many = vec!["a".to_string(), "b".to_string()];
        assert!(pick_program(&many, "c", None).is_none());
    }

    #[test]
    fn archive_icons() {
        let files: Vec<String> = [
            "browser/chrome/icons/default/default16.png",
            "browser/chrome/icons/default/default128.png",
            "browser/chrome/icons/default/default48.png",
            "browser/features/x/background.png",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            archive_icon(&files, "").unwrap(),
            "browser/chrome/icons/default/default128.png"
        );
        let named: Vec<String> = ["share/app.png", "share/icons/256x256/other.png"]
            .map(String::from)
            .to_vec();
        assert_eq!(archive_icon(&named, "app").unwrap(), "share/app.png");
        assert!(archive_icon(&["a.txt".to_string()], "").is_none());
    }

    #[test]
    fn pkgbuild_needs_a_reader() {
        // --yes picks the first choice: never "Build it".
        let q = pkgbuild_question("app", "0123", "pkgname=app");
        assert_eq!(q.choices[0].0, "no");
        assert_eq!(q.default, 0);
        assert!(q.text.contains("pkgname=app"));
    }

    #[test]
    fn ids_for_directories() {
        assert_eq!(id_from_url("", "myapp"), "org.xeeonos.app.myapp");
    }

    #[test]
    fn base_programs() {
        let closure: Vec<String> = ["glibc", "coreutils", "sed"].map(String::from).to_vec();
        let packages = vec!["tar".to_string()];
        let m = missing_base(&closure, &packages);
        assert!(m.contains(&"grep".to_string()) && m.contains(&"gzip".to_string()));
        assert!(
            !m.iter()
                .any(|p| p == "coreutils" || p == "sed" || p == "tar")
        );
        assert!(
            !BASE_TOOLS
                .iter()
                .any(|p| ["systemd", "pacman", "linux"].contains(p))
        );
    }
}
