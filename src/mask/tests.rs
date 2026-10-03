//! Unit tests for `mask`.

use super::*;

fn list(entries: &[(&str, Option<&str>)]) -> MaskList {
    MaskList {
        entries: entries
            .iter()
            .enumerate()
            .map(|(i, (pat, repo))| MaskEntry {
                pattern: pat.to_string(),
                repo: repo.map(str::to_string),
                reason: None,
                source: "test".to_string(),
                line: i + 1,
            })
            .collect(),
    }
}

#[test]
fn glob_star_matches_any_run() {
    assert!(glob_match("emacs-*", "emacs-nox"));
    assert!(glob_match("*-git", "foo-git"));
    assert!(glob_match("*", "anything"));
    assert!(!glob_match("emacs-*", "emacs"));
    assert!(!glob_match("*-git", "foo-git2"));
}

#[test]
fn bare_entry_masks_exactly_that_name() {
    let m = list(&[("emacs", None), ("emacs-*", None)]);
    assert!(m.find("emacs", None).is_some());
    assert!(m.find("extra/emacs", Some("extra")).is_some());
    assert!(m.find("emacs-nox", None).is_some());
    assert!(m.find("emacsclient", None).is_none());
}

#[test]
fn prefixed_entry_only_fires_for_that_repo() {
    let m = list(&[("nano", Some("extra"))]);
    assert!(m.find("nano", Some("extra")).is_some());
    assert!(m.find("nano", Some("aur")).is_none());
    assert!(m.find("nano", None).is_none());
}
