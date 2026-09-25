//! `/var/log/emerge.log`: append-only, human-readable record of
//! merge/unmerge events -- Portage's `emerge.log`, for pacman/AUR/ABS
//! atoms instead of ebuilds.
//!
//! Written like other root-owned `/etc/portage/*` state from this
//! unprivileged process: `sudo tee`. Best-effort -- a failed write is
//! silent, never a reason to fail (or slow down) a merge/unmerge that
//! already happened.
//!
//! One line per event:
//! ```text
//! 2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
//! 2026-09-21 08:52:10  MERGE    aur    libqalculate-5.12.0-1.1   (9s)
//! 2026-09-21 09:03:02  MERGE    extra  nano-8.0-1  vim-9.1-1     (4s)
//! 2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
//! ```
//!
//! Duration is only ever real: AUR/ABS build one at a time in this
//! tool's own loop, so each gets its own timer. A `pacman -S`/`-R`
//! batch is one exit code for the whole transaction -- no per-package
//! split, so that line carries every atom and the batch's own total.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) const LOG_FILE: &str = "/var/log/emerge.log";
const DATE_BIN: &str = "/usr/bin/date";

/// Starts a build/merge timer:
/// `let t = Timer::start(); ... log_merge_one("aur", &atom, t.elapsed());`
pub(crate) struct Timer(Instant);

impl Timer {
    pub(crate) fn start() -> Self {
        Timer(Instant::now())
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// "YYYY-MM-DD HH:MM:SS" via `date` rather than hand-rolled calendar
/// math -- this tool already shells out for everything else.
fn timestamp() -> String {
    Command::new(DATE_BIN)
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "?".to_string())
}

pub(crate) fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{}m{}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

fn append(line: &str) {
    if !crate::is_safe_path(LOG_FILE) {
        return;
    }
    let child = Command::new(crate::SUDO_BIN)
        .arg(crate::TEE_BIN)
        .arg("-a")
        .arg(LOG_FILE)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut c) = child {
        if let Some(mut stdin) = c.stdin.take() {
            let _ = writeln!(stdin, "{}", line);
        }
        let _ = c.wait();
    }
}

/// One line for a batch that installed together as a single pacman
/// transaction -- no per-package duration is meaningful there.
pub(crate) fn log_merge_batch(repo: &str, atoms: &[String], elapsed: Duration) {
    if atoms.is_empty() {
        return;
    }
    append(&format!(
        "{}  MERGE    {:<6} {}  ({})",
        timestamp(),
        repo,
        atoms.join("  "),
        fmt_duration(elapsed)
    ));
}

/// One line per package with its own real build time -- AUR/ABS,
/// which already builds one package at a time.
pub(crate) fn log_merge_one(repo: &str, atom: &str, elapsed: Duration) {
    append(&format!(
        "{}  MERGE    {:<6} {}  ({})",
        timestamp(),
        repo,
        atom,
        fmt_duration(elapsed)
    ));
}

pub(crate) fn log_unmerge(atoms: &[String]) {
    if atoms.is_empty() {
        return;
    }
    append(&format!("{}  UNMERGE  -      {}", timestamp(), atoms.join("  ")));
}

// ── `--info` stats ──────────────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq)]
pub(crate) struct LogStats {
    pub(crate) merges: usize,
    pub(crate) unmerges: usize,
    pub(crate) total_build_time: Duration,
}

/// Reparses `fmt_duration`'s own output. Kept next to it deliberately
/// -- one format, one place that knows both directions of it.
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (h, rest) = match s.split_once('h') {
        Some((h, rest)) => (h.parse().ok()?, rest),
        None => (0u64, s),
    };
    let (m, rest) = match rest.split_once('m') {
        Some((m, rest)) => (m.parse().ok()?, rest),
        None => (0u64, rest),
    };
    let secs: u64 = rest.strip_suffix('s')?.parse().ok()?;
    Some(Duration::from_secs(h * 3600 + m * 60 + secs))
}

/// Counts and sums a log's worth of lines, without caring where they
/// came from -- testable on a literal string, and `--info` just hands
/// it the file's contents. `--info` runs unprivileged, so a log with
/// restrictive permissions is a silent zero here, not an error.
pub(crate) fn parse_stats(text: &str) -> LogStats {
    let mut stats = LogStats::default();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(_date) = fields.next() else { continue };
        let Some(_time) = fields.next() else { continue };
        let Some(kind) = fields.next() else { continue };
        match kind {
            "MERGE" => {
                stats.merges += 1;
                if let Some(dur) = line.rsplit('(').next().and_then(|s| s.strip_suffix(')')) {
                    if let Some(d) = parse_duration(dur) {
                        stats.total_build_time += d;
                    }
                }
            }
            "UNMERGE" => stats.unmerges += 1,
            _ => {}
        }
    }
    stats
}

/// `--info`'s stats line: reads the log if it can, says nothing if it
/// can't (no permission, no file yet -- neither is worth a warning).
pub(crate) fn read_stats() -> Option<LogStats> {
    let text = std::fs::read_to_string(LOG_FILE).ok()?;
    Some(parse_stats(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_roundtrips_through_its_own_format() {
        for secs in [0, 9, 59, 60, 61, 3599, 3600, 3661, 7325] {
            let d = Duration::from_secs(secs);
            assert_eq!(parse_duration(&fmt_duration(d)), Some(d));
        }
    }

    #[test]
    fn stats_count_and_sum_a_realistic_log() {
        let log = "\
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 08:52:10  MERGE    aur    libqalculate-5.12.0-1.1   (9s)
2026-09-21 09:03:02  MERGE    extra  nano-8.0-1  vim-9.1-1     (1m5s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
";
        let stats = parse_stats(log);
        assert_eq!(stats.merges, 3);
        assert_eq!(stats.unmerges, 1);
        assert_eq!(stats.total_build_time, Duration::from_secs(12 + 9 + 65));
    }

    #[test]
    fn blank_and_malformed_lines_are_skipped_not_fatal() {
        let log = "\n   \nnot a log line at all\n2026-09-21 08:51:44  MERGE    aur    x-1-1  (3s)\n";
        let stats = parse_stats(log);
        assert_eq!(stats.merges, 1);
        assert_eq!(stats.total_build_time, Duration::from_secs(3));
    }

    #[test]
    fn empty_log_is_zeroed_stats_not_an_error() {
        assert_eq!(parse_stats(""), LogStats::default());
    }
}