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
//! The package is looked at in an Arch Linux container (create-arch.sh),
//! like `xapp build` does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use xapp_core::paths;
use xapp_core::ui::{self, Question};
use xapp_core::util::{Result, err, gid, uid, valid_app_id};

const ARCH_SCRIPT: &str = include_str!("create-arch.sh");

/// Where the application comes from.
pub enum Source {
    /// A package of the Arch Linux Archive snapshot.
    Arch(String),
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

/// Look at the package in the container: work/info.txt, closure.txt,
/// files.txt, pkgroot/.
fn look_at_package(work: &Path, snapshot: &str, pkg: &str) -> Result<()> {
    fs::write(work.join("create-arch.sh"), ARCH_SCRIPT)?;
    let engine = crate::engine()?;
    let mut cmd = Command::new(&engine);
    cmd.args(["run", "--rm"])
        .arg("--volume")
        .arg(format!("{}:/work", work.display()));
    if engine.ends_with("docker") {
        cmd.args(["--env", &format!("XAPP_CHOWN={}:{}", uid(), gid())]);
    }
    cmd.arg(crate::container_image())
        .args(["/bin/bash", "/work/create-arch.sh", snapshot, pkg]);
    println!("Looking at {pkg} in the Arch snapshot {snapshot} ({engine})...");
    let status = cmd
        .status()
        .map_err(|e| err(format!("cannot run {engine}: {e}")))?;
    if !status.success() {
        return Err(err(format!("could not read the package {pkg} ({status})")));
    }
    Ok(())
}

/// Make the recipe; returns its directory.
pub fn create(o: &Options) -> Result<PathBuf> {
    let Source::Arch(pkg) = &o.source;
    let snapshot = o
        .snapshot
        .clone()
        .or_else(system_snapshot)
        .ok_or_else(|| err("which Arch snapshot? (--snapshot YYYY/MM/DD)"))?;
    if xapp_core::date::parse(&snapshot).is_none() {
        return Err(err("the snapshot must be a date, YYYY/MM/DD"));
    }
    let output = o.output.clone().unwrap_or_else(|| PathBuf::from(pkg));
    if output.join("recipe.json").exists() {
        return Err(err(format!(
            "{} already has a recipe.json",
            output.display()
        )));
    }
    let work = o
        .work
        .clone()
        .unwrap_or_else(|| paths::cache_home().join("xapp/create").join(pkg));
    fs::create_dir_all(&work)?;
    let work = fs::canonicalize(&work)?;
    look_at_package(&work, &snapshot, pkg)?;

    let info = fs::read_to_string(work.join("info.txt"))?;
    let lines = |f: &str| -> Vec<String> {
        fs::read_to_string(work.join(f))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    };
    let closure = lines("closure.txt");
    let files = lines("files.txt");
    let (stem, d) = pick_desktop(&work.join("pkgroot/usr/share/applications"), pkg).ok_or_else(|| {
        err(format!(
            "{pkg} has no menu entry (.desktop): it is not an application with windows, or its entry is in another package"
        ))
    })?;
    let app_id = match &o.app_id {
        Some(id) if valid_app_id(id) => id.clone(),
        Some(id) => return Err(err(format!("\"{id}\" is not a valid application id"))),
        None => default_app_id(&stem, pkg),
    };
    let exec = command(&d.exec, &files)
        .ok_or_else(|| err(format!("the menu entry of {pkg} has no command")))?;
    // The icon: in the package, else in an icon theme (then that package
    // goes in the recipe too, unless the package already brings it).
    let mut packages = vec![pkg.clone()];
    let icon = match icon_path(&d.icon, &files) {
        Some(icon) => icon,
        None => {
            let (owner, icon) = theme_icon(&d.icon, &lines("icons.txt"), &closure).ok_or_else(|| {
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

    // The permissions: proposed, then confirmed one by one.
    let proposals = propose(&closure, &d.categories, &d.mime_types);
    let summary = format!(
        "{} {} ({app_id})\n{description}\n\nCommand: {}\nIcon: {icon}\nPackages: {} (from {})\n\n\
         The next questions propose the permissions it probably needs.",
        d.name,
        si_field(&info, "Version"),
        exec.join(" "),
        closure.len(),
        pkg
    );
    let mut questions = vec![Question::message("xapp create", &summary)];
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
    let permissions: Vec<Value> = proposals
        .iter()
        .zip(answers.iter().skip(1))
        .filter(|(_, a)| *a == "yes")
        .map(|(p, _)| json!({"id": p.id, "reason": p.reason}))
        .collect();

    let names: Map<String, Value> = d
        .names
        .iter()
        .filter(|(_, n)| *n != d.name)
        .map(|(l, n)| (l.clone(), json!(n)))
        .collect();
    let mut recipe = Map::new();
    recipe.insert("app_id".into(), json!(app_id));
    recipe.insert("name".into(), json!(d.name));
    if !names.is_empty() {
        recipe.insert("names".into(), Value::Object(names));
    }
    recipe.insert("description".into(), json!(description));
    recipe.insert("snapshot".into(), json!(snapshot));
    recipe.insert("packages".into(), json!(packages));
    recipe.insert("exec".into(), json!(exec));
    recipe.insert("gpu".into(), json!("host"));
    recipe.insert("permissions".into(), Value::Array(permissions));
    if !d.mime_types.is_empty() {
        recipe.insert("mime_types".into(), json!(d.mime_types));
    }
    if !d.categories.is_empty() {
        recipe.insert("categories".into(), json!(d.categories));
    }
    recipe.insert("wm_class".into(), json!(wm_class));
    recipe.insert("icon".into(), json!(icon));
    let mut text = serde_json::to_vec_pretty(&Value::Object(recipe)).unwrap_or_default();
    text.push(b'\n');
    fs::create_dir_all(&output)?;
    fs::write(output.join("recipe.json"), text)?;
    println!("Recipe: {}", output.join("recipe.json").display());
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
}
