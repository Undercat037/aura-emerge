//! Unit tests for `logbook` (kept out of the module file so the code stays readable).

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
