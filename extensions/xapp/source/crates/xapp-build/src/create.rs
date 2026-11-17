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
//! Both are looked at in an Arch Linux container (create-arch.sh,
//! create-url.sh), like `xapp build` does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use xapp_core::paths;
use xapp_core::ui::{self, Question};
use xapp_core::util::{Result, err, gid, uid, valid_app_id};

const ARCH_SCRIPT: &str = include_str!("create-arch.sh");
const URL_SCRIPT: &str = include_str!("create-url.sh");

/// Where the application comes from.
pub enum Source {
    /// A package of the Arch Linux Archive snapshot.
    Arch(String),
    /// An archive published by the application's authors (https://...).
    Url(String),
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
/// brings (`closure`), its categories and the file types it opens.
pub fn propose(closure: &[String], categories: &[String], mime: &[String]) -> Vec<Proposal> {
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
    let mut out = Vec::new();
    let mut add = |id: &'static str, reason: &str, because: String| {
        out.push(Proposal {
            id,
            reason: reason.to_string(),
            because,
        })
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
    out
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
    sources: Vec<Value>,
    /// More lines for the summary.
    notes: Vec<String>,
}

/// Run a script of the wizard in an Arch Linux container, on work/.
fn in_container(work: &Path, script: &str, body: &str, args: &[&str], what: &str) -> Result<()> {
    fs::write(work.join(script), body)?;
    let engine = crate::engine()?;
    let mut cmd = Command::new(&engine);
    cmd.args(["run", "--rm"])
        .arg("--volume")
        .arg(format!("{}:/work", work.display()));
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
    )?;
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
        packages,
        loads: Vec::new(),
        closure,
        sources: Vec::new(),
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
fn from_url(o: &Options, url: &str, snapshot: &str, work: &Path) -> Result<Found> {
    in_container(
        work,
        "create-url.sh",
        URL_SCRIPT,
        &[snapshot, url],
        &format!("Looking at {url} (libraries from the Arch snapshot {snapshot})"),
    )?;
    let first = |f: &str| lines_of(work, f).into_iter().next().unwrap_or_default();
    let final_url = first("url.txt");
    let sha256 = first("sha256.txt");
    if sha256.len() != 64 {
        return Err(err("the download has no SHA-256"));
    }
    let files = lines_of(work, "files.txt");
    let programs = lines_of(work, "programs.txt");
    let (file_name, file_version) = name_and_version(&url_file(&final_url));
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
        return Err(err("cannot tell the application's name from the address"));
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
                "which program starts it? Programs in the archive: {} (--exec <one of them>)",
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
            .ok_or_else(|| err("no icon in the archive (--icon <path of a .png or .svg in it>)"))?,
    };
    if !files.contains(&icon) {
        return Err(err(format!("no {icon} in the archive")));
    }

    let pkgs = |f: &str| -> Vec<String> {
        let mut v: Vec<String> = lines_of(work, f)
            .iter()
            .filter_map(|l| l.split_once(' ').map(|(_, p)| p.to_string()))
            .filter(|p| p != "glibc")
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let packages = pkgs("needs.txt");
    let loads: Vec<String> = pkgs("loads.txt")
        .into_iter()
        .filter(|p| !packages.contains(p))
        .collect();
    let missing = lines_of(work, "missing.txt");
    let mut notes = vec![
        format!("From: {final_url}"),
        format!("SHA-256: {sha256}"),
        format!("Into: {to}"),
        format!(
            "Libraries from packages: {}",
            if packages.is_empty() {
                "none".into()
            } else {
                packages.join(" ")
            }
        ),
    ];
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
        app_id: chosen_id(o)?.unwrap_or_else(|| id_from_url(url, &short)),
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
        sources: vec![json!({"url": final_url, "sha256": sha256, "to": to})],
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
    let key = match &o.source {
        Source::Arch(pkg) => pkg.clone(),
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
    let f = match &o.source {
        Source::Arch(pkg) => from_arch(o, pkg, &snapshot, &work)?,
        Source::Url(url) => from_url(o, url, &snapshot, &work)?,
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
    let proposals = propose(&f.closure, &f.categories, &f.mime_types);
    let summary = format!(
        "{} {} ({})\n{}\n\nCommand: {}\nIcon: {}\n{}\n\n\
         The next questions propose the permissions it probably needs.",
        f.name,
        f.version,
        f.app_id,
        f.description,
        f.exec.join(" "),
        f.icon,
        f.notes.join("\n"),
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
    let first_permission = questions.len();
    for p in &proposals {
        let label = xapp_permissions::info(p.id).map_or(p.id, |i| i.label);
        questions.push(Question::yes_no(
            "xapp create: permission",
            &format!(
                "{label}?\n\nWhy the wizard proposes it: {}.\nReason shown to the user: \"{}\"",
                p.because, p.reason
            ),
            "Ask for it",
            "Leave it out",
            true,
        ));
    }
    let answers =
        ui::ask(&questions).ok_or_else(|| err("nobody to answer the wizard (use --yes)"))?;
    let mut packages = f.packages.clone();
    if !f.loads.is_empty() && answers.get(1).is_some_and(|a| a == "yes") {
        packages.extend(f.loads.iter().cloned());
    }
    let permissions: Vec<Value> = proposals
        .iter()
        .zip(answers.iter().skip(first_permission))
        .filter(|(_, a)| *a == "yes")
        .map(|(p, _)| json!({"id": p.id, "reason": p.reason}))
        .collect();

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
    if !f.sources.is_empty() {
        recipe.insert("sources".into(), Value::Array(f.sources.clone()));
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
        let ids: Vec<&str> = propose(&closure, &["Utility".into()], &["image/png".into()])
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(
            ids,
            ["network", "gpu", "notifications", "audio", "pictures"]
        );
        assert!(propose(&["glibc".into()], &[], &[]).is_empty());
        // Every proposal is a known permission.
        for p in propose(
            &closure,
            &["WebBrowser".into()],
            &["text/html".into(), "video/mp4".into()],
        ) {
            assert!(xapp_permissions::info(p.id).is_some(), "{}", p.id);
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
}
