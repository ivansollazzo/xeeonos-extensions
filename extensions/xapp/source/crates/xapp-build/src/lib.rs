//! `xapp build`: making a .xapp from a recipe.
//!
//! A recipe is a directory with `recipe.json`:
//!
//! ```json
//! {
//!   "app_id": "org.mozilla.firefox",
//!   "name": "Firefox",
//!   "names": {"it": "Firefox"},
//!   "description": "Web browser",
//!   "snapshot": "2026/11/13",
//!   "packages": ["firefox"],
//!   "exec": ["/usr/lib/firefox/firefox"],
//!   "gpu": "host",
//!   "permissions": [{"id": "network", "reason": "To browse the web"}],
//!   "mime_types": ["text/html"],
//!   "categories": ["Network", "WebBrowser"],
//!   "wm_class": "firefox",
//!   "icon": "/usr/share/icons/hicolor/128x128/apps/firefox.png",
//!   "remove": ["usr/lib/firefox/updater"],
//!   "sources": [{"url": "https://.../app-1.0.tar.xz", "sha256": "...", "to": "/opt/app"}],
//!   "script": "build.sh"
//! }
//! ```
//!
//! Whether the application makes its own sandbox with user namespaces
//! (manifest `user_namespaces`) is found in the image: Chromium, Electron,
//! CEF, WebKitGTK and Qt WebEngine do. A recipe can still say
//! `"user_namespaces": true|false`.
//!
//! The capabilities (manifest `capabilities`: what it needs from the system
//! to work, given without asking) are found in the image too: "x11" for an
//! interface that opens only on X11 (needs_x11). A recipe can add one with
//! `"capabilities": ["x11"]`.
//!
//! `version` may be left out: it is then the version of the first package.
//! `sources` are archives of the application's authors (official binaries):
//! downloaded, checked against their SHA-256 and unpacked in their `to`
//! directory (/opt/<name>), after the packages.
//! The packages are installed with pacman, from the Arch Linux Archive
//! snapshot of the recipe, in an Arch Linux container (podman or docker);
//! the result becomes the erofs image of the .xapp, signed with the
//! author's key: `--key <file>`, or the key text in $XAPP_SIGNING_KEY, or
//! ~/.config/xapp/signing.key (made the first time).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use xapp_core::paths;
use xapp_core::util::{Result, err, gid, have, read_kv, run as run_program, uid};
use xapp_format::container::{self, hash_bytes, hash_file_prefix};
use xapp_format::manifest::Digest;
use xapp_format::{FORMAT, Gpu, HOST_CONTRACT, Manifest, Permission};
use xapp_trust::keys::{self, Secret};

pub mod create;

const ROOT_SCRIPT: &str = include_str!("build-root.sh");

/// Options of `xapp build`.
pub struct Options {
    pub recipe: PathBuf,
    pub key: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub work: Option<PathBuf>,
    /// A root directory already made (by other tools): packed as it is,
    /// without the container. Its packages are the recipe's business.
    pub root: Option<PathBuf>,
}

/// What the container build needs.
struct ContainerBuild<'a> {
    work: &'a Path,
    recipe_dir: &'a Path,
    icon: &'a str,
    script: &'a str,
    remove: &'a [String],
    sources: &'a [String],
    snapshot: &'a str,
    packages: &'a [String],
}

/// Make the image in an Arch Linux container (build-root.sh): work/out/.
fn image_in_container(b: &ContainerBuild, app_id: &str) -> Result<()> {
    fs::write(b.work.join("build-root.sh"), ROOT_SCRIPT)?;
    let engine = engine()?;
    let image_name = container_image();
    let mut cmd = Command::new(&engine);
    cmd.args(["run", "--rm", "--privileged"])
        .arg("--volume")
        .arg(format!("{}:/work", b.work.display()))
        .arg("--volume")
        .arg(format!("{}:/recipe:ro", b.recipe_dir.display()))
        .args(["--env", &format!("XAPP_ICON={}", b.icon)])
        .args(["--env", &format!("XAPP_SCRIPT={}", b.script)])
        .args(["--env", &format!("XAPP_REMOVE={}", b.remove.join("\n"))])
        .args(["--env", &format!("XAPP_SOURCES={}", b.sources.join("\n"))]);
    if engine.ends_with("docker") {
        // Docker's root is the real root: give the results back to the user.
        cmd.args(["--env", &format!("XAPP_CHOWN={}:{}", uid(), gid())]);
    }
    cmd.arg(&image_name)
        .args(["/bin/bash", "/work/build-root.sh", b.snapshot])
        .args(b.packages);
    println!(
        "Building {app_id} from Arch snapshot {} ({engine})...",
        b.snapshot
    );
    let status = cmd
        .status()
        .map_err(|e| err(format!("cannot run {engine}: {e}")))?;
    if !status.success() {
        return Err(err(format!("the build in the container failed ({status})")));
    }
    Ok(())
}

/// Pack a ready root directory (`--root`): work/out/.
fn image_from_root(root: &Path, recipe_dir: &Path, icon: &str, out: &Path) -> Result<()> {
    fs::create_dir_all(out)?;
    let image = out.join("image.erofs");
    let _ = fs::remove_file(&image);
    run_program(
        "mkfs.erofs",
        &[
            "--quiet",
            "-zlz4hc",
            "-T0",
            &image.to_string_lossy(),
            &root.to_string_lossy(),
        ],
    )?;
    let src = match icon.strip_prefix('/') {
        Some(inside) => root.join(inside),
        None => recipe_dir.join(icon),
    };
    let ext = icon.rsplit('.').next().unwrap_or("png");
    fs::copy(&src, out.join(format!("icon.{ext}")))
        .map_err(|e| err(format!("cannot read the icon {}: {e}", src.display())))?;
    let _ = fs::remove_file(out.join("info"));
    Ok(())
}

/// Files that tell that the application makes its own sandbox with user
/// namespaces, and would not start without them: (file name, what it is).
const OWN_SANDBOX: &[(&str, &str)] = &[
    ("chrome-sandbox", "Chromium or Electron"),
    ("v8_context_snapshot.bin", "Chromium or Electron"),
    ("libcef.so", "Chromium Embedded Framework"),
    ("QtWebEngineProcess", "Qt WebEngine"),
    ("WebKitWebProcess", "WebKitGTK"),
];

/// What makes its own sandbox among these files (paths in the image).
pub fn own_sandbox<'a>(
    files: impl IntoIterator<Item = &'a str>,
) -> Option<(&'a str, &'static str)> {
    files.into_iter().find_map(|f| {
        let name = f.rsplit('/').next().unwrap_or(f);
        OWN_SANDBOX
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, what)| (f, *what))
    })
}

/// Signs, in the image's files, of an interface that opens only on X11:
/// (file or files, why). The first that matches wins.
const X11_ONLY: &[(&[&str], &[&str], &str)] = &[
    // (all of these are there, none of these is there, why)
    (
        &["usr/lib/vlc/plugins/gui/libqt_plugin.so"],
        &[],
        "VLC 3: its Qt interface opens only on X11",
    ),
    (
        &["usr/lib/vlc/plugins/gui/libskins2_plugin.so"],
        &[],
        "VLC 3: its skins interface opens only on X11",
    ),
    (
        &["usr/lib/libgtk-x11-2.0.so.0"],
        &[],
        "GTK 2 draws only on X11",
    ),
    (
        &["usr/lib/libQt5Gui.so.5"],
        &["usr/lib/qt/plugins/platforms/libqwayland-generic.so"],
        "Qt 5 without its Wayland plugin draws on X11",
    ),
    (
        &["usr/lib/libQt6Gui.so.6"],
        &["usr/lib/qt6/plugins/platforms/libqwayland-generic.so"],
        "Qt 6 without its Wayland plugin draws on X11",
    ),
    (
        &["libawt_xawt.so"],
        &[],
        "Java (AWT, Swing) draws only on X11",
    ),
    (
        &["chrome_100_percent.pak"],
        &[],
        "Chromium or Electron: X11 unless told to use Wayland",
    ),
    (
        &["usr/lib/libX11.so.6"],
        &["usr/lib/libwayland-client.so.0"],
        "uses X11 and has no Wayland library",
    ),
];

/// Does the application need X11 to open its windows? Why, if so. A sign
/// ending with a file name only (no "/") matches that name anywhere.
pub fn needs_x11<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    let files: Vec<&str> = files.into_iter().collect();
    let has = |sign: &str| {
        files
            .iter()
            .any(|f| *f == sign || (!sign.contains('/') && f.rsplit('/').next() == Some(sign)))
    };
    X11_ONLY
        .iter()
        .find(|(all, none, _)| all.iter().all(|f| has(f)) && !none.iter().any(|f| has(f)))
        .map(|(_, _, why)| *why)
}

/// Every file under a directory, as paths relative to it.
fn files_under(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(p),
                Ok(_) => {
                    if let Ok(rel) = p.strip_prefix(root) {
                        out.push(rel.to_string_lossy().into_owned());
                    }
                }
                Err(_) => {}
            }
        }
    }
    out
}

fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The author's key, from the options, the environment or the user's own.
fn signing_key(opt: &Option<PathBuf>) -> Result<Secret> {
    if let Some(p) = opt {
        if !p.exists() {
            let k = keys::generate(p)?;
            println!(
                "New signing key: {} (public key {})",
                p.display(),
                k.public()
            );
            return Ok(k);
        }
        return keys::load(p);
    }
    if let Ok(text) = std::env::var("XAPP_SIGNING_KEY")
        && !text.trim().is_empty()
    {
        return keys::parse_secret(&text)
            .ok_or_else(|| err("XAPP_SIGNING_KEY is not an xapp secret key"));
    }
    let own = paths::config_home().join("xapp/signing.key");
    if own.exists() {
        return keys::load(&own);
    }
    let k = keys::generate(&own)?;
    println!(
        "New signing key: {} (keep it safe: updates of your applications need the same key)",
        own.display()
    );
    println!("Public key: {}", k.public());
    Ok(k)
}

/// The Arch Linux container image: $XAPP_BUILD_IMAGE, else archlinux:base.
pub(crate) fn container_image() -> String {
    std::env::var("XAPP_BUILD_IMAGE").unwrap_or_else(|_| "docker.io/library/archlinux:base".into())
}

/// The container program: $XAPP_CONTAINER, else podman, else docker.
pub(crate) fn engine() -> Result<String> {
    if let Ok(e) = std::env::var("XAPP_CONTAINER") {
        return Ok(e);
    }
    ["podman", "docker"]
        .into_iter()
        .find(|e| have(e))
        .map(String::from)
        .ok_or_else(|| {
            err("xapp build needs podman or docker (sudo xctl extensions enable podman)")
        })
}

/// "1:146.0.1-2" -> "146.0.1" (no epoch, no package release).
fn package_version(v: &str) -> String {
    let v = v.split_once(':').map_or(v, |(_, rest)| rest);
    v.rsplit_once('-').map_or(v, |(ver, _)| ver).to_string()
}

/// The recipe's `sources`, checked, as "url sha256 to" lines.
fn sources(r: &Value) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for s in r
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (url, sum, to) = (text(s, "url"), text(s, "sha256"), text(s, "to"));
        if !url.starts_with("https://") || url.contains(char::is_whitespace) {
            return Err(err(format!(
                "recipe: source \"{url}\" must be an https:// address"
            )));
        }
        if sum.len() != 64 || !sum.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err(format!("recipe: source {url} needs its sha256")));
        }
        let ok_to = to.strip_prefix("/opt/").is_some_and(|d| {
            !d.is_empty() && !d.contains("..") && !d.contains(char::is_whitespace)
        });
        if !ok_to {
            return Err(err(format!(
                "recipe: source {url} goes \"to\" a directory of /opt (/opt/<name>)"
            )));
        }
        out.push(format!("{url} {} {to}", sum.to_lowercase()));
    }
    Ok(out)
}

/// Build the .xapp; returns its path.
pub fn build(o: &Options) -> Result<PathBuf> {
    let recipe_path = o.recipe.join("recipe.json");
    let r: Value = serde_json::from_slice(
        &fs::read(&recipe_path)
            .map_err(|e| err(format!("cannot read {}: {e}", recipe_path.display())))?,
    )
    .map_err(|e| err(format!("{} is not valid JSON: {e}", recipe_path.display())))?;
    let snapshot = text(&r, "snapshot");
    if xapp_core::date::parse(&snapshot).is_none() {
        return Err(err("recipe: snapshot must be a date, YYYY/MM/DD"));
    }
    let app_id = text(&r, "app_id");
    if !xapp_core::util::valid_app_id(&app_id) {
        return Err(err(format!(
            "recipe: \"{app_id}\" is not a valid application id"
        )));
    }
    let packages = list(&r, "packages");
    let icon = text(&r, "icon");
    let icon_type = icon.rsplit('.').next().unwrap_or_default().to_lowercase();
    if !matches!(icon_type.as_str(), "png" | "svg") {
        return Err(err("recipe: icon must be a .png or .svg file"));
    }
    for p in r
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = text(p, "id");
        // X11 was a permission once: now a capability (below).
        if xapp_permissions::info(&id).is_none() && id != "x11" {
            return Err(err(format!(
                "recipe: unknown permission \"{id}\" (known: {})",
                xapp_permissions::known()
            )));
        }
    }
    let sources = sources(&r)?;
    let key = signing_key(&o.key)?;

    // 1. The image, made in a container.
    let work = o
        .work
        .clone()
        .unwrap_or_else(|| paths::cache_home().join("xapp/build").join(&app_id));
    fs::create_dir_all(&work)?;
    let work = fs::canonicalize(&work)?;
    let recipe_dir = fs::canonicalize(&o.recipe)?;
    let out = work.join("out");
    match &o.root {
        Some(root) => image_from_root(root, &recipe_dir, &icon, &out)?,
        None => {
            let c = ContainerBuild {
                work: &work,
                recipe_dir: &recipe_dir,
                icon: &icon,
                script: &text(&r, "script"),
                remove: &list(&r, "remove"),
                sources: &sources,
                snapshot: &snapshot,
                packages: &packages,
            };
            image_in_container(&c, &app_id)?;
        }
    }
    let image = out.join("image.erofs");
    let files = match &o.root {
        Some(root) => files_under(root),
        None => fs::read_to_string(out.join("files.txt"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect(),
    };
    let found = own_sandbox(files.iter().map(String::as_str));
    let user_namespaces = match r.get("user_namespaces").and_then(Value::as_bool) {
        Some(v) => v,
        None => found.is_some(),
    };
    if let Some((file, what)) = found {
        println!(
            "{what} inside ({file}): its own sandbox {}",
            if user_namespaces {
                "is allowed"
            } else {
                "is not allowed (recipe)"
            }
        );
    }
    // Capabilities: from the recipe, then from the image.
    let mut capabilities = list(&r, "capabilities");
    for c in &capabilities {
        if xapp_permissions::capability(c).is_none() {
            return Err(err(format!("recipe: unknown capability \"{c}\"")));
        }
    }
    let x11_permission = r
        .get("permissions")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|p| text(p, "id") == "x11"));
    if x11_permission && !capabilities.iter().any(|c| c == "x11") {
        capabilities.push("x11".into());
    }
    if !capabilities.iter().any(|c| c == "x11")
        && let Some(why) = needs_x11(files.iter().map(String::as_str))
    {
        println!("X11 windows given ({why})");
        capabilities.push("x11".into());
    }
    let info = read_kv(&out.join("info"));
    let info = |k: &str| {
        info.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| text(&r, k))
    };

    // 2. The manifest.
    container::stamp_image(&image)?;
    let image_size = fs::metadata(&image)?.len();
    let icon_bytes = fs::read(out.join(format!("icon.{icon_type}")))?;
    let version = match text(&r, "version") {
        v if !v.is_empty() => v,
        _ => package_version(&info("main")),
    };
    let names = r
        .get("names")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let permissions = r
        .get("permissions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|p| text(p, "id") != "x11")
                .map(|p| Permission {
                    id: text(p, "id"),
                    reason: text(p, "reason"),
                })
                .collect()
        })
        .unwrap_or_default();
    let m = Manifest {
        format: FORMAT,
        app_id,
        name: text(&r, "name"),
        names,
        description: text(&r, "description"),
        version,
        exec: list(&r, "exec"),
        arch: "x86_64".into(),
        snapshot: snapshot.replace('/', "-"),
        host_contract: HOST_CONTRACT,
        gpu: if text(&r, "gpu") == "bundled" {
            Gpu::Bundled
        } else {
            Gpu::Host
        },
        user_namespaces,
        capabilities,
        glibc: info("glibc"),
        libstdcxx: info("libstdcxx"),
        permissions,
        mime_types: list(&r, "mime_types"),
        categories: list(&r, "categories"),
        wm_class: text(&r, "wm_class"),
        icon_type: icon_type.clone(),
        icon: Digest {
            size: icon_bytes.len() as u64,
            sha256: hash_bytes(&icon_bytes),
        },
        image: Digest {
            size: image_size,
            sha256: hash_file_prefix(&image, image_size)?,
        },
        signer: key.public(),
    };
    m.check()?;
    let manifest_bytes = m.to_json();
    let signature = key.sign(&manifest_bytes);

    // 3. The file.
    let output = o
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.xapp", file_name(&m.name))));
    container::write(&output, &image, &manifest_bytes, &signature, &icon_bytes)?;
    let _ = fs::remove_file(&image);
    println!(
        "Built {} {}: {} ({} MB), signed by {}",
        m.app_id,
        m.version,
        output.display(),
        fs::metadata(&output)?.len() / 1_000_000,
        keys::fingerprint(&m.signer)
    );
    Ok(output)
}

/// A file name from an application name ("Text Editor" stays, "/" goes).
pub fn file_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if c == '/' || c.is_control() { '-' } else { c })
        .collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() {
        "application".into()
    } else {
        n
    }
}

/// Where `xapp install` and `xapp build` put a file: ~/Applications/<name>.
pub fn install_path(source: &Path, name: &str) -> PathBuf {
    let base = source
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .filter(|f| f.to_lowercase().ends_with(".xapp"))
        .unwrap_or_else(|| format!("{}.xapp", file_name(name)));
    paths::applications().join(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_names() {
        assert_eq!(package_version("146.0.1-2"), "146.0.1");
        assert_eq!(package_version("1:2.0-1"), "2.0");
        assert_eq!(file_name("Text Editor"), "Text Editor");
        assert_eq!(file_name("a/b"), "a-b");
        assert_eq!(file_name(".."), "application");
    }

    #[test]
    fn finds_own_sandboxes() {
        let files = ["usr/bin/bash", "usr/lib/electron39/chrome-sandbox"];
        assert_eq!(
            own_sandbox(files),
            Some(("usr/lib/electron39/chrome-sandbox", "Chromium or Electron"))
        );
        assert_eq!(
            own_sandbox(["usr/lib/firefox/firefox", "usr/bin/gimp"]),
            None
        );
        assert!(own_sandbox(["usr/lib/webkitgtk-6.0/WebKitWebProcess"]).is_some());
        // A name, not a part of one.
        assert_eq!(own_sandbox(["usr/share/doc/chrome-sandbox.txt"]), None);
    }

    #[test]
    fn finds_x11_only_interfaces() {
        let vlc = [
            "usr/bin/vlc",
            "usr/lib/libX11.so.6",
            "usr/lib/libwayland-client.so.0",
            "usr/lib/vlc/plugins/gui/libqt_plugin.so",
        ];
        assert!(needs_x11(vlc).unwrap().contains("VLC"));
        // GTK 3/4 and Qt with its Wayland plugin: Wayland.
        let gtk = [
            "usr/lib/libX11.so.6",
            "usr/lib/libwayland-client.so.0",
            "usr/lib/libgtk-4.so.1",
        ];
        assert_eq!(needs_x11(gtk), None);
        let qt6 = [
            "usr/lib/libX11.so.6",
            "usr/lib/libwayland-client.so.0",
            "usr/lib/libQt6Gui.so.6",
            "usr/lib/qt6/plugins/platforms/libqwayland-generic.so",
        ];
        assert_eq!(needs_x11(qt6), None);
        assert!(needs_x11(["usr/lib/libQt6Gui.so.6", "usr/lib/libwayland-client.so.0"]).is_some());
        // A program with only X11.
        assert!(needs_x11(["usr/bin/xterm", "usr/lib/libX11.so.6"]).is_some());
        // Electron, anywhere.
        assert!(
            needs_x11([
                "opt/App/chrome_100_percent.pak",
                "usr/lib/libwayland-client.so.0"
            ])
            .is_some()
        );
        // Nothing graphical.
        assert_eq!(needs_x11(["usr/bin/bash"]), None);
    }

    #[test]
    fn recipe_sources() {
        let sum = "a".repeat(64);
        let ok = serde_json::json!({"sources": [{"url": "https://x.org/a.tar.xz", "sha256": sum, "to": "/opt/a"}]});
        assert_eq!(
            sources(&ok).unwrap(),
            [format!("https://x.org/a.tar.xz {sum} /opt/a")]
        );
        for bad in [
            serde_json::json!({"sources": [{"url": "http://x.org/a", "sha256": sum, "to": "/opt/a"}]}),
            serde_json::json!({"sources": [{"url": "https://x.org/a", "sha256": "12", "to": "/opt/a"}]}),
            serde_json::json!({"sources": [{"url": "https://x.org/a", "sha256": sum, "to": "/usr/a"}]}),
            serde_json::json!({"sources": [{"url": "https://x.org/a", "sha256": sum, "to": "/opt/../etc"}]}),
        ] {
            assert!(sources(&bad).is_err(), "{bad}");
        }
        assert!(sources(&serde_json::json!({})).unwrap().is_empty());
    }
}
