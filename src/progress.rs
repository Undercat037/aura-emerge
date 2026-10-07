//! Shared `>>> Verb (n of m) atom` status lines + live Jobs footer.
//!
//! One run-wide counter so repo, AUR and ABS stages number consistently
//! (`(15 of 16)` after 14 repo packages). The Jobs line is rewritten in
//! place (no extra newline) so it stays at the bottom of the TTY until
//! `finish()`. Load avg refreshes on a background tick while a job runs.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use colored::Colorize;

static TOTAL: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static RUNNING: AtomicUsize = AtomicUsize::new(0);
/// A live `>>> Jobs:` line occupies the current TTY row (no trailing newline).
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Background Load-avg ticker is running.
static TICKING: AtomicBool = AtomicBool::new(false);
/// Serializes all Jobs / stage line I/O across worker threads.
static OUT: Mutex<()> = Mutex::new(());

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
    start_ticker();
}

/// Make room for `n` more packages if the plan did not cover them.
pub(crate) fn reserve(n: usize) {
    let need = NEXT.load(Ordering::Relaxed) + n;
    if TOTAL.load(Ordering::Relaxed) < need {
        TOTAL.store(need, Ordering::Relaxed);
    }
    start_ticker();
}

/// A dependency discovered mid-run (AUR deps are found after clone).
pub(crate) fn grow(n: usize) {
    TOTAL.fetch_add(n, Ordering::Relaxed);
}

/// Next 1-based package number.
pub(crate) fn take() -> usize {
    NEXT.fetch_add(1, Ordering::Relaxed) + 1
}

/// Drop one RUNNING slot without printing Completed (failed install).
pub(crate) fn abort_one() {
    let _ = RUNNING.try_update(Ordering::Relaxed, Ordering::Relaxed, |r| {
        Some(r.saturating_sub(1))
    });
}

/// `atom` is `repo/name-version` (or bare `name-version`).
/// Erase Jobs → print stage line → redraw Jobs on the next row (in place).
pub(crate) fn line(stage: Stage, n: usize, atom: &str) {
    match stage {
        // Installing opens a slot; Compiling keeps it; Completed closes it.
        Stage::Installing => {
            RUNNING.fetch_add(1, Ordering::Relaxed);
        }
        Stage::Compiling => {}
        Stage::Completed => {
            let _ = RUNNING.try_update(Ordering::Relaxed, Ordering::Relaxed, |r| {
                Some(r.saturating_sub(1))
            });
            DONE.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
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

/// Remove the live Jobs line so other output can print cleanly.
/// Call `status_resume()` afterwards (or the next `line`/`finish`).
pub(crate) fn status_break() {
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
}

/// Put the Jobs line back at the bottom after `status_break` + other output.
pub(crate) fn status_resume() {
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_draw();
}

/// One-shot note above the Jobs line: erase → print → redraw Jobs.
pub(crate) fn note(text: &str) {
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
    println!("{}", text);
    status_draw();
}

/// End of run: leave one final Jobs line in the scrollback (with newline).
/// Idempotent: a second call is a no-op.
pub(crate) fn finish() {
    stop_ticker();
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
    let total = TOTAL.swap(0, Ordering::Relaxed);
    if total == 0 {
        return;
    }
    let done = DONE.load(Ordering::Relaxed);
    println!("{}", jobs_text(done, total, 0));
    SHOWN.store(false, Ordering::Relaxed);
}

/// Clear the Jobs row in place (Jobs was drawn without a trailing newline).
fn status_erase() {
    if SHOWN.swap(false, Ordering::Relaxed) {
        print!("\r\x1b[2K");
        let _ = std::io::stdout().flush();
    }
}

/// Draw Jobs on the current row without a newline so it stays the last line.
fn status_draw() {
    if !std::io::stdout().is_terminal() || crate::runtime::get().debug {
        return;
    }
    let total = TOTAL.load(Ordering::Relaxed);
    if total == 0 {
        return;
    }
    let done = DONE.load(Ordering::Relaxed);
    let running = RUNNING.load(Ordering::Relaxed);
    // \r\x1b[2K: rewrite this row; no \n — cursor stays on the Jobs line.
    print!("\r\x1b[2K{}", jobs_text(done, total, running));
    let _ = std::io::stdout().flush();
    SHOWN.store(true, Ordering::Relaxed);
}

fn start_ticker() {
    if !std::io::stdout().is_terminal() || crate::runtime::get().debug {
        return;
    }
    if TICKING.swap(true, Ordering::Relaxed) {
        return; // already running
    }
    thread::spawn(|| {
        while TICKING.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(750));
            if !TICKING.load(Ordering::Relaxed) {
                break;
            }
            // Only refresh while a Jobs line is live.
            if SHOWN.load(Ordering::Relaxed) {
                let total = TOTAL.load(Ordering::Relaxed);
                if total == 0 {
                    continue;
                }
                let done = DONE.load(Ordering::Relaxed);
                let running = RUNNING.load(Ordering::Relaxed);
                // Rewrite in place under OUT so we don't race with line().
                if let Ok(_g) = OUT.try_lock() {
                    if SHOWN.load(Ordering::Relaxed) {
                        print!("\r\x1b[2K{}", jobs_text(done, total, running));
                        let _ = std::io::stdout().flush();
                    }
                }
            }
        }
    });
}

fn stop_ticker() {
    TICKING.store(false, Ordering::Relaxed);
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
