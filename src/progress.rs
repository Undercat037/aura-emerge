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
use crate::theme::Themed;

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
    /// Fetch / resolve / start of work (repo extract, AUR clone).
    Emerging,
    /// Active makepkg / compile (AUR/ABS only; repos skip this).
    Installing,
    Completed,
}

impl Stage {
    fn word(self) -> &'static str {
        match self {
            Stage::Emerging => "Emerging",
            Stage::Installing => "Installing",
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
        // Emerging opens a slot; Installing keeps it; Completed closes it.
        Stage::Emerging => {
            RUNNING.fetch_add(1, Ordering::Relaxed);
        }
        Stage::Installing => {}
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
        ">>>".t_green().bold(),
        stage.word(),
        n.to_string().t_yellow().bold(),
        total.to_string().t_yellow().bold(),
        atom.t_green().bold()
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

/// Handle a helper `hook …` event, keeping the Jobs footer pinned.
///
///   `hook start pre|post`           → `>>> Running pre/post-transaction hooks...`
///   `hook run N/M name [desc…]`     → `>>> (N of M) desc`
///   `hook done …`                   → no-op
pub(crate) fn on_hook_event(ev: &str) {
    let mut it = ev.split_whitespace();
    let (Some("hook"), Some(kind)) = (it.next(), it.next()) else {
        return;
    };
    match kind {
        "start" => {
            let when = it.next().unwrap_or("post");
            let label = if when == "pre" {
                "pre-transaction"
            } else {
                "post-transaction"
            };
            note(&format!(
                "{} Running {} hooks...",
                ">>>".t_green().bold(),
                label
            ));
        }
        "run" => {
            // `hook run 1/3 name Optional description words…`
            let Some(frac) = it.next() else {
                return;
            };
            let (pos, total) = frac
                .split_once('/')
                .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
                .unwrap_or((0, 0));
            let name = it.next().unwrap_or("?");
            let desc: String = it.collect::<Vec<_>>().join(" ");
            let shown = if desc.is_empty() { name } else { desc.as_str() };
            note(&format!(
                "{} ({} of {}) {}",
                ">>>".t_green().bold(),
                pos.to_string().t_yellow().bold(),
                total.to_string().t_yellow().bold(),
                shown
            ));
        }
        _ => {}
    }
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
    if !std::io::stdout().is_terminal()
        || crate::runtime::get().debug
        || crate::runtime::get().nospinner
    {
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
    if !std::io::stdout().is_terminal()
        || crate::runtime::get().debug
        || crate::runtime::get().nospinner
    {
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
        ">>>".t_green().bold(),
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

    let star = ">>>".t_green().bold();
    println!("{} Waiting {} seconds before starting...", star, secs);
    println!("{} (Control-C to abort)...", star);
    print!("{} {} in:", star, what);
    let _ = std::io::stdout().flush();
    for i in (1..=secs).rev() {
        print!(" {}", i.to_string().t_red().bold());
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
