//! Dialogs: questions to the user (open an application from an unknown
//! author? let it use the microphone?).
//!
//! They are terminal dialogs, drawn by `whiptail` like xctl's (black and
//! white). Where the question is asked:
//!
//!   1. in the terminal xapp runs in, if there is one;
//!   2. otherwise (application started from the menu or with a double
//!      click), in a terminal window opened for the purpose (GNOME Console):
//!      xapp writes the questions to a file in $XDG_RUNTIME_DIR/xapp/dialogs,
//!      the window runs `xapp dialog <file>`, which writes the answers next
//!      to it, and the first xapp reads them;
//!   3. with nobody to ask (no terminal, no window), there is no answer:
//!      the caller treats it as "no".
//!
//! `XAPP_DIALOG=text` asks with plain text lines instead of whiptail (tests,
//! simple terminals); `XAPP_TERMINAL` names the terminal program for 2
//! (`none`: never open one); `--yes` (or `XAPP_ASSUME_YES=1`) answers every
//! question with its first choice (scripts, CI).

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_like::{decode_list, encode_list};

use crate::paths;
use crate::util::{Result, err, have, write_atomic};

/// whiptail's colors: white on black, as in xctl.
const COLORS: &str = "root=white,black;border=white,black;window=white,black;\
shadow=black,black;title=white,black;button=black,white;actbutton=black,white;\
compactbutton=white,black;checkbox=white,black;actcheckbox=black,white;\
entry=white,black;disentry=white,black;label=white,black;listbox=white,black;\
actlistbox=black,white;sellistbox=black,white;actsellistbox=black,white;\
textbox=white,black;acttextbox=black,white;emptyscale=white,black;\
fullscale=black,white;helpline=white,black;roottext=white,black";

/// How long a dialog window may wait for the user.
const WINDOW_WAIT: Duration = Duration::from_secs(600);

static ASSUME_YES: AtomicBool = AtomicBool::new(false);

/// Answer every question with its first choice (`--yes`).
pub fn assume_yes() {
    ASSUME_YES.store(true, Ordering::Relaxed);
}

fn assuming_yes() -> bool {
    ASSUME_YES.load(Ordering::Relaxed)
        || std::env::var("XAPP_ASSUME_YES").is_ok_and(|v| v == "1" || v == "yes")
}

/// One question. The first choice is the "yes" one (what `--yes` picks).
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub title: String,
    pub text: String,
    /// (key, label): the key is the answer, the label is shown.
    pub choices: Vec<(String, String)>,
    /// The choice selected at first (index in `choices`).
    pub default: usize,
    /// A list to tick (`checklist`): whether each choice starts ticked.
    /// Empty: an ordinary question (one choice).
    pub checked: Vec<bool>,
    /// What `--yes` answers, when not the first choice.
    pub yes: Option<String>,
}

impl Question {
    /// A yes/no question; `default_yes` says which button is selected.
    pub fn yes_no(title: &str, text: &str, yes: &str, no: &str, default_yes: bool) -> Self {
        Question {
            title: title.into(),
            text: text.into(),
            choices: vec![("yes".into(), yes.into()), ("no".into(), no.into())],
            default: if default_yes { 0 } else { 1 },
            checked: Vec::new(),
            yes: None,
        }
    }

    /// A message with only an OK button.
    pub fn message(title: &str, text: &str) -> Self {
        Question {
            title: title.into(),
            text: text.into(),
            choices: vec![("ok".into(), "OK".into())],
            default: 0,
            checked: Vec::new(),
            yes: None,
        }
    }

    /// A list to tick, like tasksel: (key, label, ticked at first). The
    /// answer is the ticked keys' positions (`ticked`); `--yes` keeps the
    /// first ticks, unless `yes` says otherwise.
    pub fn checklist(title: &str, text: &str, items: &[(String, String, bool)]) -> Self {
        Question {
            title: title.into(),
            text: text.into(),
            choices: items
                .iter()
                .map(|(k, l, _)| (k.clone(), l.clone()))
                .collect(),
            default: 0,
            checked: items.iter().map(|(_, _, on)| *on).collect(),
            yes: None,
        }
    }

    /// Is it a list to tick?
    pub fn is_checklist(&self) -> bool {
        !self.checked.is_empty()
    }

    /// The answer when the user ticks these positions.
    pub fn ticks_answer(ticked: &[usize]) -> String {
        if ticked.is_empty() {
            "-".into()
        } else {
            ticked
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }
    }

    /// The answer --yes gives.
    fn yes_answer(&self) -> String {
        if let Some(y) = &self.yes {
            return y.clone();
        }
        if self.is_checklist() {
            let on: Vec<usize> = (0..self.checked.len())
                .filter(|&i| self.checked[i])
                .collect();
            return Question::ticks_answer(&on);
        }
        self.choices[0].0.clone()
    }
}

/// The positions ticked in a checklist's answer.
pub fn ticked(answer: &str) -> Vec<usize> {
    answer
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect()
}

/// Ask the questions, in order. `None`: there is nobody to ask.
pub fn ask(questions: &[Question]) -> Option<Vec<String>> {
    if questions.is_empty() {
        return Some(Vec::new());
    }
    if assuming_yes() {
        return Some(questions.iter().map(Question::yes_answer).collect());
    }
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        return ask_here(questions);
    }
    ask_in_window(questions)
}

/// Ask on this terminal.
pub fn ask_here(questions: &[Question]) -> Option<Vec<String>> {
    let text_mode = text_mode();
    questions
        .iter()
        .map(|q| match (text_mode, q.is_checklist()) {
            (true, true) => ask_text_checklist(q),
            (true, false) => ask_text(q),
            (false, true) => ask_whiptail_checklist(q),
            (false, false) => ask_whiptail(q),
        })
        .collect()
}

/// Plain text, a list to tick: numbers toggle, an empty line confirms.
fn ask_text_checklist(q: &Question) -> Option<String> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut on = q.checked.clone();
    loop {
        let _ = writeln!(out, "\n== {} ==\n{}\n", q.title, q.text);
        for (i, (_, label)) in q.choices.iter().enumerate() {
            let mark = if on[i] { "x" } else { " " };
            let _ = writeln!(out, "  [{mark}] {}) {label}", i + 1);
        }
        let _ = write!(out, "Numbers to tick or untick, Enter to confirm> ");
        let _ = out.flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        if line.is_empty() {
            let ticked: Vec<usize> = (0..on.len()).filter(|&i| on[i]).collect();
            return Some(Question::ticks_answer(&ticked));
        }
        for word in line.split(|c: char| c == ',' || c.is_whitespace()) {
            if let Ok(n) = word.parse::<usize>()
                && (1..=on.len()).contains(&n)
            {
                on[n - 1] = !on[n - 1];
            }
        }
    }
}

/// whiptail, a list to tick (like tasksel).
fn ask_whiptail_checklist(q: &Question) -> Option<String> {
    let height = q.choices.len().min(12).to_string();
    let mut cmd = Command::new("whiptail");
    cmd.env("NEWT_COLORS", COLORS).args([
        "--backtitle",
        "XeeonOS",
        "--title",
        &q.title,
        "--notags",
        "--separate-output",
        "--checklist",
        &q.text,
        "22",
        "78",
        &height,
    ]);
    for (i, (_, label)) in q.choices.iter().enumerate() {
        cmd.args([
            i.to_string(),
            label.clone(),
            if q.checked[i] { "ON" } else { "OFF" }.to_string(),
        ]);
    }
    let out = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if out.status.code() != Some(0) {
        return None;
    }
    let ticked: Vec<usize> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .filter(|&n: &usize| n < q.choices.len())
        .collect();
    Some(Question::ticks_answer(&ticked))
}

/// A line of text from the user, on this terminal (`None`: nobody to ask,
/// or cancelled). `--yes` answers `default`.
pub fn input(title: &str, text: &str, default: &str) -> Option<String> {
    if assuming_yes() {
        return Some(default.to_string());
    }
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return None;
    }
    if text_mode() {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "\n== {title} ==\n{text}");
        if !default.is_empty() {
            let _ = writeln!(out, "(Enter: {default})");
        }
        let _ = write!(out, "> ");
        let _ = out.flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        return Some(if line.is_empty() { default } else { line }.to_string());
    }
    let out = Command::new("whiptail")
        .env("NEWT_COLORS", COLORS)
        .args([
            "--backtitle",
            "XeeonOS",
            "--title",
            title,
            "--inputbox",
            text,
            "12",
            "76",
            default,
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    (out.status.code() == Some(0)).then(|| String::from_utf8_lossy(&out.stderr).trim().to_string())
}

fn text_mode() -> bool {
    std::env::var("XAPP_DIALOG").is_ok_and(|v| v == "text")
        || !have("whiptail")
        || std::env::var("TERM").map_or(true, |t| t == "dumb")
}

/// Plain text: the question, numbered choices, a line to answer.
fn ask_text(q: &Question) -> Option<String> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    loop {
        let _ = writeln!(out, "\n== {} ==\n{}\n", q.title, q.text);
        for (i, (_, label)) in q.choices.iter().enumerate() {
            let mark = if i == q.default { " (default)" } else { "" };
            let _ = writeln!(out, "  {}) {label}{mark}", i + 1);
        }
        let _ = write!(out, "> ");
        let _ = out.flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).ok()? == 0 {
            return None; // end of input
        }
        let line = line.trim();
        if line.is_empty() {
            return Some(q.choices[q.default].0.clone());
        }
        if let Ok(n) = line.parse::<usize>()
            && (1..=q.choices.len()).contains(&n)
        {
            return Some(q.choices[n - 1].0.clone());
        }
        // Also the key itself, or the first letter of a label (y/n).
        for (key, label) in &q.choices {
            let first = label.chars().next().map(|c| c.to_ascii_lowercase());
            if line.eq_ignore_ascii_case(key)
                || (line.len() == 1 && first == line.chars().next().map(|c| c.to_ascii_lowercase()))
            {
                return Some(key.clone());
            }
        }
    }
}

/// whiptail: a yes/no window for two choices, a menu for more.
fn ask_whiptail(q: &Question) -> Option<String> {
    let mut cmd = Command::new("whiptail");
    cmd.env("NEWT_COLORS", COLORS)
        .args(["--backtitle", "XeeonOS", "--title", &q.title]);
    let menu = q.choices.len() != 2;
    if menu {
        let height = (q.choices.len() + 2).to_string();
        cmd.args([
            "--notags",
            "--default-item",
            &q.choices[q.default].0,
            "--menu",
            &q.text,
            "20",
            "76",
            &height,
        ]);
        for (key, label) in &q.choices {
            cmd.args([key, label]);
        }
    } else {
        cmd.args([
            "--yes-button",
            &q.choices[0].1,
            "--no-button",
            &q.choices[1].1,
        ]);
        if q.default == 1 {
            cmd.arg("--defaultno");
        }
        // A long text (a PKGBUILD) scrolls.
        cmd.args(["--scrolltext", "--yesno", &q.text, "20", "76"]);
    }
    let out = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    let code = out.status.code();
    if menu {
        // The chosen key is written on standard error; Esc/Cancel: nothing.
        let key = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if code == Some(0) && q.choices.iter().any(|(k, _)| *k == key) {
            return Some(key);
        }
        // A single OK button counts as answered either way.
        return (q.choices.len() == 1).then(|| q.choices[0].0.clone());
    }
    match code {
        Some(0) => Some(q.choices[0].0.clone()),
        Some(1) => Some(q.choices[1].0.clone()),
        _ => None,
    }
}

/// The terminal program for dialog windows, as a command to put the xapp
/// command after (`None`: there is none, or `XAPP_TERMINAL=none`).
fn terminal() -> Option<Vec<String>> {
    if let Ok(t) = std::env::var("XAPP_TERMINAL") {
        let words: Vec<String> = t.split_whitespace().map(String::from).collect();
        return (!words.is_empty() && words[0] != "none").then_some(words);
    }
    // Without a graphical session there is no window to open.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return None;
    }
    let known: &[&[&str]] = &[
        &["kgx", "--"],
        &["ptyxis", "--"],
        &["gnome-terminal", "--"],
        &["xterm", "-e"],
    ];
    known
        .iter()
        .find(|t| have(t[0]))
        .map(|t| t.iter().map(|s| s.to_string()).collect())
}

/// Ask in a terminal window opened for the purpose.
fn ask_in_window(questions: &[Question]) -> Option<Vec<String>> {
    let terminal = terminal()?;
    let dir = paths::runtime().join("dialogs");
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let request = dir.join(format!("{id}.questions"));
    let answers = dir.join(format!("{id}.answers"));
    let started = dir.join(format!("{id}.pid"));
    write_atomic(&request, encode_list(questions).as_bytes()).ok()?;
    let exe = std::env::current_exe().ok()?;
    let spawned = Command::new(&terminal[0])
        .args(&terminal[1..])
        .arg(exe)
        .arg("dialog")
        .arg(&request)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(_) => {
            let _ = std::fs::remove_file(&request);
            return None;
        }
    };
    let start = Instant::now();
    let result = loop {
        if let Ok(text) = std::fs::read_to_string(&answers) {
            break decode_answers(&text, questions.len());
        }
        // The window runs `xapp dialog`, which writes its process id when it
        // starts: once it has started and is gone without answers, the window
        // was closed.
        if let Some(pid) = std::fs::read_to_string(&started)
            .ok()
            .map(|s| s.trim().to_string())
            && !Path::new("/proc").join(&pid).exists()
            && !answers.exists()
        {
            break None;
        }
        if start.elapsed() > WINDOW_WAIT {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = child.try_wait();
    for f in [&request, &answers, &started] {
        let _ = std::fs::remove_file(f);
    }
    result
}

fn decode_answers(text: &str, n: usize) -> Option<Vec<String>> {
    let answers: Vec<String> = text.lines().map(|l| l.trim().to_string()).collect();
    (answers.len() == n && answers.iter().all(|a| !a.is_empty())).then_some(answers)
}

/// `xapp dialog <file>`: the side of the dialog window. Asks the questions
/// of the file on this terminal and writes the answers next to it.
pub fn answer_file(request: &Path) -> Result<()> {
    let text = std::fs::read_to_string(request)?;
    let questions = decode_list(&text).ok_or_else(|| err("not a dialog file"))?;
    let base = request.with_extension("");
    write_atomic(
        &base.with_extension("pid"),
        std::process::id().to_string().as_bytes(),
    )?;
    let answers = ask_here(&questions).ok_or_else(|| err("no answer"))?;
    write_atomic(
        &base.with_extension("answers"),
        answers.join("\n").as_bytes(),
    )?;
    Ok(())
}

/// The dialog file: one question per block, fields one per line, escaped so
/// that a line never contains a line break. Plain text, no JSON library
/// needed in this crate.
mod serde_like {
    use super::Question;

    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\n', "\\n")
    }

    fn unesc(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    pub fn encode_list(questions: &[Question]) -> String {
        let mut out = String::new();
        for q in questions {
            out.push_str(&format!(
                "title={}\ntext={}\ndefault={}\n",
                esc(&q.title),
                esc(&q.text),
                q.default
            ));
            for (key, label) in &q.choices {
                out.push_str(&format!("choice={}\t{}\n", esc(key), esc(label)));
            }
            if !q.checked.is_empty() {
                let marks: String = q
                    .checked
                    .iter()
                    .map(|c| if *c { '1' } else { '0' })
                    .collect();
                out.push_str(&format!("checked={marks}\n"));
            }
            out.push_str("end\n");
        }
        out
    }

    pub fn decode_list(text: &str) -> Option<Vec<Question>> {
        let mut out = Vec::new();
        let empty = || Question {
            title: String::new(),
            text: String::new(),
            choices: Vec::new(),
            default: 0,
            checked: Vec::new(),
            yes: None,
        };
        let mut q = empty();
        for line in text.lines() {
            if line == "end" {
                if q.choices.is_empty()
                    || q.default >= q.choices.len()
                    || (!q.checked.is_empty() && q.checked.len() != q.choices.len())
                {
                    return None;
                }
                out.push(std::mem::replace(&mut q, empty()));
                continue;
            }
            let (key, value) = line.split_once('=')?;
            match key {
                "title" => q.title = unesc(value),
                "text" => q.text = unesc(value),
                "default" => q.default = value.parse().ok()?,
                "checked" => q.checked = value.chars().map(|c| c == '1').collect(),
                "choice" => {
                    let (k, l) = value.split_once('\t')?;
                    q.choices.push((unesc(k), unesc(l)));
                }
                _ => return None,
            }
        }
        Some(out)
    }
}

/// The path of a dialog file's answers (for tests and the dialog side).
pub fn answers_path(request: &Path) -> PathBuf {
    request.with_extension("answers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_file_round_trip() {
        let qs = vec![
            Question::yes_no(
                "Open?",
                "Line one\nline two = \\ ok",
                "Open",
                "Cancel",
                false,
            ),
            Question::message("Refused", "tab\there"),
            Question::checklist(
                "Allow?",
                "Tick",
                &[
                    ("camera".into(), "Camera".into(), false),
                    ("audio".into(), "Sound".into(), true),
                ],
            ),
        ];
        let text = encode_list(&qs);
        assert_eq!(decode_list(&text), Some(qs));
        assert_eq!(
            decode_answers("yes\nok\n", 2),
            Some(vec!["yes".into(), "ok".into()])
        );
        assert_eq!(decode_answers("yes\n", 2), None);
    }

    #[test]
    fn checklists() {
        let q = Question::checklist(
            "t",
            "x",
            &[
                ("a".into(), "A".into(), true),
                ("b".into(), "B".into(), false),
                ("c".into(), "C".into(), true),
            ],
        );
        assert!(q.is_checklist());
        assert_eq!(q.yes_answer(), "0,2");
        assert_eq!(ticked("0,2"), [0, 2]);
        assert!(ticked("-").is_empty());
        let mut q = q;
        q.yes = Some("-".into());
        assert_eq!(q.yes_answer(), "-");
    }
}
