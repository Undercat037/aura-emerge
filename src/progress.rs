//! Shared `>>> Verb (n of m, p%) atom` status lines + live Jobs footer.
//!
//! One run-wide counter so repo, AUR and ABS stages number consistently.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::theme::Themed;
use colored::Colorize;

static TOTAL: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static RUNNING: AtomicUsize = AtomicUsize::new(0);
/// Live `>>> Jobs:` line occupies the current TTY row (no trailing newline).
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Interactive prompt is on screen: suppress Jobs redraw until resume.
static HELD: AtomicBool = AtomicBool::new(false);
/// Rows the live area occupies (fetch rows + Jobs row).
static ROWS: AtomicUsize = AtomicUsize::new(1);
/// Background Load-avg ticker is running.
static TICKING: AtomicBool = AtomicBool::new(false);
/// Serializes Jobs / stage line I/O across worker threads.
static OUT: Mutex<()> = Mutex::new(());
/// Packages being fetched: (n, atom, percent).
static FETCH: Mutex<Vec<(usize, String, u8)>> = Mutex::new(Vec::new());
/// Wall-clock start of the current run (for elapsed in Jobs line).
static START: Mutex<Option<Instant>> = Mutex::new(None);
/// Last drawn Jobs text (ticker skips redraw if nothing visible changed).
static LAST_JOBS: Mutex<String> = Mutex::new(String::new());

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
    if let Ok(mut s) = START.lock() {
        *s = Some(Instant::now());
    }
    if let Ok(mut last) = LAST_JOBS.lock() {
        last.clear();
    }
    fetch_clear();
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
    stage_line(stage, n, atom, None);
}

/// Emerging line with the data-received percent (download progress).
pub(crate) fn emerging(n: usize, atom: &str, pct: u8) {
    stage_line(Stage::Emerging, n, atom, Some(pct));
}

fn stage_line(stage: Stage, n: usize, atom: &str, dl_pct: Option<u8>) {
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
    let count = format!(
        "{} of {}",
        n.to_string().t_yellow().bold(),
        total.to_string().t_yellow().bold()
    );
    // Per-package progress (download / clone / install of *this* atom).
    // Live fetch rows already show the running %; permanent lines usually
    // appear at 100% or when the stage advances.
    let head = match stage {
        Stage::Emerging => {
            let p = dl_pct.unwrap_or(100);
            format!("({}, {}%)", count, p.to_string().t_cyan().bold())
        }
        Stage::Installing | Stage::Completed => format!("({})", count),
    };
    println!(
        "{} {} {} {}",
        ">>>".t_green().bold(),
        stage.word(),
        head,
        atom.t_green().bold()
    );
    status_draw();
}

/// Remove the live Jobs line so other output (scanner prompt, ask) can print.
/// Holds the ticker until `status_resume`.
pub(crate) fn status_break() {
    HELD.store(true, Ordering::Relaxed);
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
}

/// Put the Jobs line back after `status_break` + interactive output.
pub(crate) fn status_resume() {
    HELD.store(false, Ordering::Relaxed);
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
///   `hook start pre|post` → `>>> Running pre/post-transaction hooks...`
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
    fetch_clear();
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
    let total = TOTAL.swap(0, Ordering::Relaxed);
    if total == 0 {
        return;
    }
    let done = DONE.load(Ordering::Relaxed);
    println!("{}", jobs_text(done, total, 0));
    SHOWN.store(false, Ordering::Relaxed);
    if let Ok(mut s) = START.lock() {
        *s = None;
    }
    if let Ok(mut last) = LAST_JOBS.lock() {
        last.clear();
    }
}

/// Clear the whole live area (fetch rows + Jobs row), cursor ends on its top row.
fn status_erase() {
    if SHOWN.swap(false, Ordering::Relaxed) {
        let mut out = String::from("\r\x1b[2K");
        for _ in 1..ROWS.load(Ordering::Relaxed) {
            out.push_str("\x1b[1A\r\x1b[2K");
        }
        print!("{}", out);
        let _ = std::io::stdout().flush();
    }
}

/// Draw one row per active fetch, then Jobs; no trailing newline so Jobs
/// stays the last line.
fn status_draw() {
    if HELD.load(Ordering::Relaxed)
        || !std::io::stdout().is_terminal()
        || crate::runtime::get().debug
        || crate::runtime::get().nospinner
    {
        return;
    }
    let total = TOTAL.load(Ordering::Relaxed);
    if total == 0 {
        return;
    }
    let rows = fetch_rows(total);
    let fetching = FETCH.lock().unwrap_or_else(|e| e.into_inner()).len();
    let jobs = jobs_text(
        DONE.load(Ordering::Relaxed),
        total,
        RUNNING.load(Ordering::Relaxed) + fetching,
    );
    let mut out = String::new();
    for r in &rows {
        out.push_str("\r\x1b[2K");
        out.push_str(r);
        out.push('\n');
    }
    out.push_str("\r\x1b[2K");
    out.push_str(&jobs);
    print!("{}", out);
    let _ = std::io::stdout().flush();
    ROWS.store(rows.len() + 1, Ordering::Relaxed);
    SHOWN.store(true, Ordering::Relaxed);
    if let Ok(mut last) = LAST_JOBS.lock() {
        *last = jobs;
    }
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
            // 2s is enough for elapsed / load; avoids TTY thrash.
            thread::sleep(Duration::from_millis(2000));
            if !TICKING.load(Ordering::Relaxed) {
                break;
            }
            if !SHOWN.load(Ordering::Relaxed) {
                continue;
            }
            let total = TOTAL.load(Ordering::Relaxed);
            if total == 0 {
                continue;
            }
            // Only redraw when the Jobs line text actually changed
            // (elapsed second, load, or counts). No spam.
            let fetching = FETCH.lock().unwrap_or_else(|e| e.into_inner()).len();
            let candidate = jobs_text(
                DONE.load(Ordering::Relaxed),
                total,
                RUNNING.load(Ordering::Relaxed) + fetching,
            );
            let changed = LAST_JOBS
                .lock()
                .map(|last| *last != candidate)
                .unwrap_or(true);
            if !changed {
                continue;
            }
            if let Ok(_g) = OUT.try_lock()
                && SHOWN.load(Ordering::Relaxed)
            {
                status_erase();
                status_draw();
            }
        }
    });
}

fn stop_ticker() {
    TICKING.store(false, Ordering::Relaxed);
}

/// One colored row per active fetch (by package number, at most 12).
fn fetch_rows(total: usize) -> Vec<String> {
    let mut f: Vec<(usize, String, u8)> = FETCH.lock().unwrap_or_else(|e| e.into_inner()).clone();
    f.sort_by_key(|e| e.0);
    let cols = term_cols();
    f.into_iter()
        .take(12)
        .map(|(n, atom, pct)| {
            let t = total.max(n);
            let plain = format!(">>> Emerging ({} of {}, {}%) ", n, t, pct);
            let room = cols.saturating_sub(plain.chars().count() + 1);
            let atom: String = if atom.chars().count() > room {
                atom.chars()
                    .take(room.saturating_sub(1))
                    .collect::<String>()
                    + "…"
            } else {
                atom
            };
            format!(
                "{} {} ({} of {}, {}%) {}",
                ">>>".t_green().bold(),
                Stage::Emerging.word(),
                n.to_string().t_yellow().bold(),
                t.to_string().t_yellow().bold(),
                pct.to_string().t_cyan().bold(),
                atom.t_green().bold()
            )
        })
        .collect()
}

/// Update the live fetch row of package `n` (TTY only).
/// Skips the redraw when the percent did not change (avoids TTY thrash).
pub(crate) fn fetch(n: usize, atom: &str, pct: u8) {
    let changed = {
        let mut f = FETCH.lock().unwrap_or_else(|e| e.into_inner());
        match f.iter_mut().find(|e| e.0 == n) {
            Some(e) => {
                if e.2 == pct {
                    false
                } else {
                    e.2 = pct;
                    true
                }
            }
            None => {
                f.push((n, atom.to_string(), pct));
                true
            }
        }
    };
    if !changed {
        return;
    }
    let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    status_erase();
    status_draw();
}

fn fetch_active(n: usize) -> bool {
    let f = FETCH.lock().unwrap_or_else(|e| e.into_inner());
    f.iter().any(|e| e.0 == n)
}

fn fetch_done(n: usize) {
    let mut f = FETCH.lock().unwrap_or_else(|e| e.into_inner());
    f.retain(|e| e.0 != n);
}

/// Drop stale fetch rows (failed / aborted transaction).
pub(crate) fn fetch_clear() {
    FETCH.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// `name` from `name-ver-rel-arch.pkg.tar.zst` ("" if not a package file).
fn pkg_of_file(file: &str) -> String {
    if !file.contains(".pkg.tar") {
        return String::new();
    }
    let stem = file.split(".pkg.tar").next().unwrap_or(file);
    let mut p = stem.rsplitn(4, '-');
    let _ = (p.next(), p.next(), p.next());
    p.next().unwrap_or("").to_string()
}

/// Helper `dl <file> <pct>`: live Emerging row; at 100% it becomes a
/// normal line. `nums` maps name -> package number (shared with pkg events).
pub(crate) fn on_dl_event(
    ev: &str,
    nums: &mut HashMap<String, usize>,
    atoms: &HashMap<String, String>,
) {
    let mut it = ev.split_whitespace();
    let (Some("dl"), Some(file), Some(p)) = (it.next(), it.next(), it.next()) else {
        return;
    };
    let Ok(pct) = p.parse::<u8>() else {
        return;
    };
    let name = pkg_of_file(file);
    if name.is_empty() {
        return;
    }
    let atom = atoms.get(&name).cloned().unwrap_or_else(|| name.clone());
    let n = match nums.get(&name) {
        Some(&n) => n,
        None => {
            if !atoms.contains_key(&name) {
                grow(1); // dependency, not in the plan
            }
            let n = take();
            nums.insert(name.clone(), n);
            n
        }
    };
    if pct >= 100 {
        fetch_done(n);
        emerging(n, &atom, 100);
    } else {
        fetch(n, &atom, pct);
    }
}

/// Helper `pkg start <name>`: Emerging if the fetch did not already print it.
/// Returns the package number.
pub(crate) fn on_pkg_start(
    name: &str,
    nums: &mut HashMap<String, usize>,
    atoms: &HashMap<String, String>,
) -> usize {
    let atom = atoms.get(name).cloned().unwrap_or_else(|| name.to_string());
    match nums.get(name).copied() {
        Some(n) => {
            if fetch_active(n) {
                fetch_done(n);
                emerging(n, &atom, 100);
            }
            n
        }
        None => {
            if !atoms.contains_key(name) {
                grow(1); // dependency, not in the plan
            }
            let n = take();
            nums.insert(name.to_string(), n);
            emerging(n, &atom, 100); // cached: data already here
            n
        }
    }
}

/// `>>> Jobs: 0 of 136 complete, 3 running, 3 merge wait   1m23s   Load avg: ...`
fn jobs_text(done: usize, total: usize, running: usize) -> String {
    let wait = total.saturating_sub(done + running);
    let mut left = format!("Jobs: {} of {} complete", done, total);
    if running > 0 {
        left.push_str(&format!(", {} running", running));
    }
    if wait > 0 {
        left.push_str(&format!(", {} merge wait", wait));
    }
    let elapsed = elapsed_str();
    let load = load_avg();
    // ">>> " prefix is 4 columns; elapsed + load right-aligned.
    let mid = if elapsed.is_empty() {
        String::new()
    } else {
        format!("  {}", elapsed)
    };
    let pad = term_cols()
        .saturating_sub(4 + left.chars().count() + mid.chars().count() + load.chars().count());
    format!(
        "{} {}{}{}{}",
        ">>>".t_green().bold(),
        left,
        mid,
        " ".repeat(pad.max(2)),
        load
    )
}

fn elapsed_str() -> String {
    let Ok(guard) = START.lock() else {
        return String::new();
    };
    let Some(start) = *guard else {
        return String::new();
    };
    let secs = start.elapsed().as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
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
