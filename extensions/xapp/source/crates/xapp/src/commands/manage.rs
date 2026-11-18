//! install, remove, list, info, verify, permissions, logs.

use std::fs;

use xapp_core::paths;
use xapp_core::ui::{self, Question};
use xapp_core::util::{Result, err, run as run_program};
use xapp_desktop::{registry, sync::files};
use xapp_format::Container;
use xapp_permissions::{self as perms, State, overrides};
use xapp_sandbox::instances;
use xapp_sandbox::trace::{self, Finding};
use xapp_trust::{self as trust, Trust, keys};

use super::{app_id_of, open_target};

/// Check a file before anything is done with it.
fn checked(path: &str) -> Result<(Container, Trust)> {
    let c = Container::open(std::path::Path::new(path))?;
    let t = trust::check(&c)?;
    trust::check_image_cached(&c)?;
    Ok((c, t))
}

/// `xapp install <file>`: copy into ~/Applications (the monitor does the
/// rest; here it is done at once).
pub fn install(args: &[String]) -> Result<u8> {
    let Some(file) = args.first() else {
        return Err(err("usage: xapp install <file.xapp>"));
    };
    let (c, t) = checked(file)?;
    let dest = xapp_build::install_path(&c.path, &c.manifest.name);
    if fs::canonicalize(&c.path).ok() != fs::canonicalize(&dest).ok() {
        fs::create_dir_all(paths::applications())?;
        // Copied under a hidden name, then renamed: the monitor never sees
        // half a file.
        let tmp = dest.with_file_name(format!(".{}.part", c.manifest.app_id));
        fs::copy(&c.path, &tmp)?;
        fs::rename(&tmp, &dest)?;
    }
    xapp_desktop::sync()?;
    println!(
        "Installed {} {}: {}",
        c.manifest.display_name(),
        c.manifest.version,
        dest.display()
    );
    if t == Trust::Unknown {
        println!(
            "Unknown author ({}): you will be asked when it first opens.",
            keys::fingerprint(&c.manifest.signer)
        );
    }
    Ok(0)
}

/// `xapp remove <app> [--purge]`.
pub fn remove(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err("usage: xapp remove <app> [--purge]"));
    };
    let purge = args.iter().any(|a| a == "--purge");
    let id = app_id_of(target)?;
    let mut removed = 0;
    for f in files() {
        if Container::open(&f).is_ok_and(|c| c.manifest.app_id == id) {
            fs::remove_file(&f)?;
            println!("Removed {}", f.display());
            removed += 1;
        }
    }
    xapp_desktop::sync()?;
    let data = paths::app_data(&id);
    if purge {
        if data.exists() {
            fs::remove_dir_all(&data)?;
        }
        perms::forget(&id);
        trust::policy::forget(&id);
        registry::forget(&id);
        println!("Data, permissions and key of {id} deleted.");
    } else if data.exists() {
        println!(
            "Its data stays in {} (xapp remove {id} --purge deletes it).",
            data.display()
        );
    }
    if removed == 0 && !purge {
        return Err(err(format!(
            "{id} is not in {}",
            paths::applications().display()
        )));
    }
    Ok(0)
}

/// `xapp list`.
pub fn list() -> Result<u8> {
    let report = xapp_desktop::sync()?;
    let entries = registry::all();
    if entries.is_empty() {
        println!("No applications in {}.", paths::applications().display());
    }
    let trusted = trust::policy::trusted_keys();
    for e in &entries {
        let author = if trusted.contains(&e.signer) {
            "trusted author"
        } else {
            "author not verified"
        };
        println!("{:<24} {:<32} {:<14} {author}", e.name, e.app_id, e.version);
    }
    for (f, why) in &report.skipped {
        println!("Left out: {} ({why})", f.display());
    }
    Ok(0)
}

/// `xapp info <app|file>`.
pub fn info(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err("usage: xapp info <app|file>"));
    };
    let c = open_target(target)?;
    let m = &c.manifest;
    let state = trust::check(&c);
    let known = matches!(state, Ok(Trust::Trusted | Trust::Approved));
    println!("{} {}", m.display_name(), m.version);
    println!("Id:          {}", m.app_id);
    if !m.description.is_empty() {
        println!("Description: {}", m.description);
    }
    println!(
        "File:        {} ({} MB)",
        c.path.display(),
        fs::metadata(&c.path)?.len() / 1_000_000
    );
    let author = match &state {
        Ok(Trust::Trusted) => "trusted".to_string(),
        Ok(Trust::Approved) => "not verified; opened anyway before".to_string(),
        Ok(Trust::Unknown) => "not verified; asked when it opens".to_string(),
        Err(e) => format!("REFUSED: {e}"),
    };
    println!("Author:      {} ({author})", keys::fingerprint(&m.signer));
    if m.user_namespaces {
        println!("Sandbox:     makes its own inside (user namespaces)");
    }
    if m.multilib {
        println!("Programs:    32-bit too");
    }
    for cap in &m.capabilities {
        let label = perms::capability(cap).map_or("unknown to this xapp", |c| c.label);
        println!("Needs:       {label}");
    }
    let extra = overrides::load(&m.app_id);
    if !extra.is_empty() {
        println!("Added by hand (xapp doctor):");
        for (kind, value) in extra.entries() {
            let what = overrides::KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or(kind, |(_, w)| w);
            println!("  {what}: {value}");
        }
    }
    let age = xapp_core::date::parse(&m.snapshot).map(|d| xapp_core::date::today() - d);
    let old = match age {
        Some(days) if days > 365 => format!(
            " (more than {} months old: its libraries may lack security fixes)",
            days / 30
        ),
        _ => String::new(),
    };
    println!("Based on:    Arch Linux snapshot {}{old}", m.snapshot);
    println!(
        "Graphics:    {}",
        if m.gpu == xapp_format::Gpu::Host {
            "drivers of the system"
        } else {
            "its own drivers"
        }
    );
    println!("Data:        {}", paths::app_data(&m.app_id).display());
    let all = perms::all(m, known);
    if all.is_empty() {
        println!("Permissions: none");
    } else {
        println!("Permissions:");
        for (p, s) in all {
            let s = match s {
                State::Yes => "allowed",
                State::No => "not allowed",
                State::Ask => "asked when it opens",
            };
            let label = if perms::valid(&p.id) {
                perms::label(&p.id)
            } else {
                "unknown to this xapp".to_string()
            };
            println!("  {:<14} {:<20} {label}: \"{}\"", p.id, s, p.reason);
        }
    }
    Ok(0)
}

/// Names on the session bus that open much more than one application:
/// the doctor warns before giving them.
fn sensitive(name: &str) -> Option<&'static str> {
    perms::sensitive_bus_name(name)
}

/// `xapp doctor <app> [--reset]`: offer what the last traced run
/// (`xapp run --trace`) was refused, one question each.
pub fn doctor(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err("usage: xapp doctor <app> [--reset]"));
    };
    let c = open_target(target)?;
    let m = &c.manifest;
    let name = m.display_name().to_string();
    if args.iter().any(|a| a == "--reset") {
        overrides::reset(&m.app_id);
        println!("{name}: what was added by hand is gone; it has only what its file says.");
        return Ok(0);
    }
    let Some(report) = trace::load(&m.app_id) else {
        println!(
            "{name} was not traced yet. Start it with:\n  xapp run --trace {}",
            m.app_id
        );
        return Ok(1);
    };
    println!(
        "Last traced run of {name}: ended with code {} after {} s.",
        report.code, report.seconds
    );
    let extra = overrides::load(&m.app_id);
    let known = matches!(trust::check(&c), Ok(Trust::Trusted | Trust::Approved));
    // What a permission gives: the permission is offered (below), also if
    // the application does not declare it (then it is added by hand).
    let mut wanted: Vec<String> = Vec::new();
    for f in &report.findings {
        if let Finding::Wants(p) = f
            && perms::state(m, p, known) != State::Yes
        {
            if m.declares(p).is_none() {
                println!(
                    "{name} {}, but its file does not ask for it (its recipe should).",
                    f.describe()
                );
            }
            wanted.push(p.clone());
        }
    }
    let open: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|f| {
            let (kind, value) = f.entry();
            !matches!(f, Finding::Wants(_))
                && !extra.has(kind, value)
                && !(**f == Finding::X11 && perms::has_capability(m, "x11"))
        })
        .collect();
    let mut refused: Vec<String> = wanted;
    for (p, s) in perms::all(m, known) {
        if s == State::No && perms::valid(&p.id) && !refused.contains(&p.id) {
            refused.push(p.id.clone());
        }
    }
    if open.is_empty() && refused.is_empty() {
        println!(
            "The sandbox refused nothing it could give. Its own messages: xapp logs {}",
            m.app_id
        );
        return Ok(0);
    }
    let mut questions = Vec::new();
    for f in &open {
        let mut text = format!("{name} {}.", f.describe());
        let default_yes = match f {
            Finding::Own(n) | Finding::Talk(n) | Finding::See(n) => match sensitive(n) {
                Some(what) => {
                    text.push_str(&format!("\n\nCareful: {n} gives access to {what}."));
                    false
                }
                None => true,
            },
            Finding::X11 => {
                text.push_str("\n\nIts windows are then less isolated from other X11 windows.");
                true
            }
            Finding::Wants(_) => true,
        };
        text.push_str("\n\nUndo with: xapp doctor ");
        text.push_str(&m.app_id);
        text.push_str(" --reset");
        let mut q = Question::yes_no(
            &format!("{name}: allow?"),
            &text,
            "Allow",
            "Don't allow",
            default_yes,
        );
        if !default_yes {
            // "Don't allow" first: what --yes answers.
            q.choices.reverse();
            q.default = 0;
        }
        questions.push(q);
    }
    for id in &refused {
        let label = perms::label(id);
        let mut text = format!("{name} may not use: {label}.\n\nAllow it now?");
        if let Some(w) = perms::warning(id) {
            text.push_str(&format!("\n\nCareful: {w}"));
        }
        let mut q = Question::yes_no(
            &format!("{name}: permission"),
            &text,
            "Allow",
            "Don't allow",
            false,
        );
        if perms::warning(id).is_some() {
            q.choices.reverse();
            q.default = 0;
        }
        questions.push(q);
    }
    let Some(answers) = ui::ask(&questions) else {
        return Err(err("no terminal to ask in (use --yes)"));
    };
    let mut changed = 0;
    for (f, a) in open.iter().zip(&answers) {
        if a == "yes" {
            let (kind, value) = f.entry();
            overrides::add(&m.app_id, kind, value)?;
            changed += 1;
        }
    }
    for (id, a) in refused.iter().zip(&answers[open.len()..]) {
        if a == "yes" {
            perms::set(&m.app_id, id, Some(true))?;
            changed += 1;
        }
    }
    println!("{name}: {changed} change(s). They apply from its next start.");
    Ok(0)
}

/// `xapp verify <file>`: the whole check, the image read again.
pub fn verify(args: &[String]) -> Result<u8> {
    let Some(file) = args.first() else {
        return Err(err("usage: xapp verify <file.xapp>"));
    };
    let c = Container::open(std::path::Path::new(file))?;
    let t = trust::check(&c)?;
    c.check_image()?;
    let author = match t {
        Trust::Trusted => "trusted author",
        Trust::Approved => "author not verified (opened anyway before)",
        Trust::Unknown => "author not verified",
    };
    println!(
        "{}: {} {} is intact, signed by {} ({author}).",
        file,
        c.manifest.app_id,
        c.manifest.version,
        keys::fingerprint(&c.manifest.signer)
    );
    Ok(0)
}

/// "yes", "no", "ask" and their synonyms.
fn choice(word: &str) -> Result<Option<bool>> {
    match word {
        "yes" | "allow" | "on" => Ok(Some(true)),
        "no" | "deny" | "off" => Ok(Some(false)),
        "ask" | "reset" => Ok(None),
        other => Err(err(format!("\"{other}\": say yes, no or ask"))),
    }
}

/// Before giving a permission to handle with care: say why.
fn warn_if_careful(id: &str, value: Option<bool>) {
    if value == Some(true)
        && let Some(w) = perms::warning(id)
    {
        eprintln!("xapp: careful: {w}");
    }
}

const PERMISSIONS_USAGE: &str = "usage: xapp permissions <app> [<permission> yes|no|ask | --reset]\n       \
     xapp permissions --all [<permission> yes|no|ask]";

/// `xapp permissions <app> [<permission> yes|no|ask | --reset]`, and
/// `xapp permissions --all [...]` for every application. Like `flatpak
/// override`: the user may also give a permission the application does not
/// declare, or take back one it does.
pub fn permissions(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err(PERMISSIONS_USAGE));
    };
    if target == "--all" {
        return match args.len() {
            1 => {
                let all = perms::global_choices();
                if all.is_empty() {
                    println!("No choice for every application.");
                }
                for (id, v) in all {
                    println!(
                        "{id:<14} {:<4} {}",
                        if v { "yes" } else { "no" },
                        perms::label(&id)
                    );
                }
                Ok(0)
            }
            3 => {
                let value = choice(&args[2])?;
                warn_if_careful(&args[1], value);
                perms::set_global(&args[1], value)?;
                println!(
                    "Every application: {} {}",
                    args[1],
                    match value {
                        Some(true) => "allowed (unless you said no for one of them)",
                        Some(false) => "not allowed (unless you said yes for one of them)",
                        None => "as each application asks",
                    }
                );
                let _ = xapp_desktop::sync();
                Ok(0)
            }
            _ => Err(err(PERMISSIONS_USAGE)),
        };
    }
    if args.get(1).is_some_and(|a| a == "--reset") {
        let id = app_id_of(target)?;
        perms::forget(&id);
        println!("{id}: your choices are gone; it asks again at the next start.");
        let _ = xapp_desktop::sync();
        return Ok(0);
    }
    if args.len() >= 3 {
        let id = app_id_of(target)?;
        let value = choice(&args[2])?;
        warn_if_careful(&args[1], value);
        perms::set(&id, &args[1], value)?;
        println!(
            "{id}: {} {}",
            args[1],
            match value {
                Some(true) => "allowed",
                Some(false) => "not allowed",
                None => "as it asks (at the next start)",
            }
        );
        let _ = xapp_desktop::sync();
        return Ok(0);
    }
    if args.len() != 1 {
        return Err(err(PERMISSIONS_USAGE));
    }
    let c = open_target(target)?;
    let known = matches!(trust::check(&c), Ok(Trust::Trusted | Trust::Approved));
    let all = perms::all(&c.manifest, known);
    if all.is_empty() {
        println!("{} asks for no permissions.", c.manifest.app_id);
    }
    for (p, s) in all {
        let s = match s {
            State::Yes => "yes",
            State::No => "no",
            State::Ask => "ask",
        };
        println!("{:<14} {s:<4} {}", p.id, p.reason);
    }
    Ok(0)
}

/// `xapp ps`: the applications running now.
pub fn ps() -> Result<u8> {
    let all = instances::running();
    if all.is_empty() {
        println!("No application is running.");
        return Ok(0);
    }
    println!(
        "{:<32} {:>8} {:>12} {:>9}",
        "APPLICATION", "PID", "RUNNING", "MEMORY"
    );
    for r in &all {
        println!(
            "{:<32} {:>8} {:>12} {:>9}",
            r.app_id,
            r.pid,
            instances::human_duration(r.uptime),
            instances::memory(r).map_or("-".to_string(), instances::human_size)
        );
    }
    Ok(0)
}

/// `xapp kill <app>`: stop every instance of it.
pub fn kill(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err("usage: xapp kill <app>"));
    };
    let id = app_id_of(target)?;
    let mine: Vec<_> = instances::running()
        .into_iter()
        .filter(|r| r.app_id == id)
        .collect();
    if mine.is_empty() {
        println!("{id} is not running.");
        return Ok(1);
    }
    for r in &mine {
        instances::stop(r)?;
    }
    println!("{id} stopped.");
    Ok(0)
}

/// `xapp logs <app> [-f]`.
pub fn logs(args: &[String]) -> Result<u8> {
    let Some(target) = args.first() else {
        return Err(err("usage: xapp logs <app> [-f]"));
    };
    let id = app_id_of(target)?;
    let ident = format!("xapp-{id}");
    let mut cmd = vec!["--user", "--no-pager", "-t", ident.as_str()];
    if args.iter().any(|a| a == "-f" || a == "--follow") {
        cmd.push("-f");
    }
    run_program("journalctl", &cmd)?;
    Ok(0)
}
