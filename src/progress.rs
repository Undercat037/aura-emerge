//! Shared `>>> Verb (n of m) atom` status lines.
//!
//! One run-wide counter so repo, AUR and ABS stages number consistently
//! (`(15 of 16)` after 14 repo packages). Lines are printed at real
//! stage boundaries only.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use colored::Colorize;

static TOTAL: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static RUNNING: AtomicUsize = AtomicUsize::new(0);
/// A live `>>> Jobs:` line is on screen right above the cursor.
static SHOWN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    /// Fetching / cloning / writing files.
    Installing,
    /// Active makepkg run (AUR/ABS/local).
    Compiling,
    Completed,
}

impl Stage {
    fn word(self) -> &'static str {
        match self {
            Stage::Installing => "Installing",
            Stage::Compiling => "Compiling",
            Stage::Completed => "Completed",
        }
    }
}

/// Start a run with a known plan size.
pub(crate) fn begin(total: usize) {
    TOTAL.store(total, Ordering::Relaxed);
    NEXT.store(0, Ordering::Relaxed);
    DONE.store(0, Ordering::Relaxed);
    RUNNING.store(0, Ordering::Relaxed);
    SHOWN.store(false, Ordering::Relaxed);
}

/// Make room for `n` more packages if the plan did not cover them.
pub(crate) fn reserve(n: usize) {
    let need = NEXT.load(Ordering::Relaxed) + n;
    if TOTAL.load(Ordering::Relaxed) < need {
        TOTAL.store(need, Ordering::Relaxed);
    }
}

/// A dependency discovered mid-run (AUR deps are found after clone).
pub(crate) fn grow(n: usize) {
    TOTAL.fetch_add(n, Ordering::Relaxed);
}

/// Next 1-based package number.
pub(crate) fn take() -> usize {
    NEXT.fetch_add(1, Ordering::Relaxed) + 1
}

/// `atom` is `repo/name-version` (or bare `name-version`).
/// On a TTY the Jobs line is kept right below the newest line: erased,
/// the line printed, Jobs redrawn. Not pinned - it scrolls with the output.
pub(crate) fn line(stage: Stage, n: usize, atom: &str) {
    match stage {
        Stage::Installing | Stage::Compiling => RUNNING.store(1, Ordering::Relaxed),
        Stage::Completed => {
            RUNNING.store(0, Ordering::Relaxed);
            DONE.fetch_add(1, Ordering::Relaxed);
        }
    }
    status_erase();
    let total = TOTAL.load(Ordering::Relaxed).max(n);
    println!(
        "{} {} ({} of {}) {}",
        ">>>".green().bold(),
        stage.word(),
        n.to_string().yellow().bold(),
        total.to_string().yellow().bold(),
        atom.green().bold()
    );
    status_draw();
}

/// Remove the live Jobs line (call before printing anything else).
pub(crate) fn status_break() {
    status_erase();
}

/// End of run: leave one final Jobs line in the scrollback.
pub(crate) fn finish() {
    status_erase();
    let total = TOTAL.load(Ordering::Relaxed);
    if total == 0 {
        return;
    }
    let done = DONE.load(Ordering::Relaxed);
    println!("{}", jobs_text(done, total, 0));
}

fn status_erase() {
    if SHOWN.swap(false, Ordering::Relaxed) {
        print!("\x1b[1A\r\x1b[2K");
        let _ = std::io::stdout().flush();
    }
}

fn status_draw() {
    if !std::io::stdout().is_terminal() || crate::runtime::get().debug {
        return;
    }
    let total = TOTAL.load(Ordering::Relaxed);
    let done = DONE.load(Ordering::Relaxed);
    let running = RUNNING.load(Ordering::Relaxed);
    println!("{}", jobs_text(done, total, running));
    let _ = std::io::stdout().flush();
    SHOWN.store(true, Ordering::Relaxed);
}

/// `>>> Jobs: 0 of 136 complete, 3 running, 3 merge wait      Load avg: ...`
fn jobs_text(done: usize, total: usize, running: usize) -> String {
    let wait = total.saturating_sub(done + running);
    let mut left = format!("Jobs: {} of {} complete", done, total);
    if running > 0 {
        left.push_str(&format!(", {} running", running));
    }
    if wait > 0 {
        left.push_str(&format!(", {} merge wait", wait));
    }
    let load = load_avg();
    // ">>> " prefix is 4 columns; load avg is right-aligned.
    let pad = term_cols().saturating_sub(4 + left.chars().count() + load.chars().count());
    format!(
        "{} {}{}{}",
        ">>>".green().bold(),
        left,
        " ".repeat(pad.max(2)),
        load
    )
}

fn load_avg() -> String {
    let raw = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let v: Vec<&str> = raw.split_whitespace().take(3).collect();
    if v.len() < 3 {
        return String::new();
    }
    let f = |s: &str| format!("{:.2}", s.parse::<f64>().unwrap_or(0.0));
    format!("Load avg: {}, {}, {}", f(v[0]), f(v[1]), f(v[2]))
}

fn term_cols() -> usize {
    // SAFETY: plain ioctl on stdout into a zeroed winsize.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0;
    if ok && ws.ws_col > 0 {
        ws.ws_col as usize
    } else {
        80
    }
}

/// True only the first time `key` is seen in this run (one-shot notes).
pub(crate) fn once(key: &'static str) -> bool {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut seen) = SEEN.lock() else {
        return true;
    };
    if seen.contains(&key) {
        return false;
    }
    seen.push(key);
    true
}

/// Portage-style abort window:
/// `>>> Waiting 5 seconds before starting...` / `>>> Unmerging in: 5 4 3 2 1`.
pub(crate) fn countdown(what: &str, secs: u32) {
    use std::io::Write;

    let star = ">>>".green().bold();
    println!("{} Waiting {} seconds before starting...", star, secs);
    println!("{} (Control-C to abort)...", star);
    print!("{} {} in:", star, what);
    let _ = std::io::stdout().flush();
    for i in (1..=secs).rev() {
        print!(" {}", i.to_string().red().bold());
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!();
}

/// `repo/name-version`, dropping empty parts.
pub(crate) fn atom(repo: &str, name: &str, version: &str) -> String {
    let mut s = String::new();
    if !repo.is_empty() {
        s.push_str(repo);
        s.push('/');
    }
    s.push_str(name);
    if !version.is_empty() {
        s.push('-');
        s.push_str(version);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_formats() {
        assert_eq!(atom("extra", "mpv", "1:0.41.0-6"), "extra/mpv-1:0.41.0-6");
        assert_eq!(atom("", "mpv", "1-1"), "mpv-1-1");
        assert_eq!(atom("aur", "foo", ""), "aur/foo");
    }
}
