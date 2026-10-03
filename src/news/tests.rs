//! Unit tests for `news` (kept out of the module file so the code stays readable).

use super::*;

#[test]
fn sanitize_strips_ansi_and_bidi() {
    let dirty = "hello\x1b[2J\x1b]0;pwned\x07 world\u{202E}reversed";
    let clean = sanitize_text(dirty);
    assert!(!clean.contains('\x1b'));
    assert!(!clean.contains('\u{202E}'));
    assert!(clean.contains("hello"));
    assert!(clean.contains("world"));
}

#[test]
fn sanitize_collapses_newlines_so_guid_stays_one_line() {
    let forged = "legit-guid\nextra-forged-guid";
    assert_eq!(sanitize_text(forged), "legit-guid extra-forged-guid");
}
