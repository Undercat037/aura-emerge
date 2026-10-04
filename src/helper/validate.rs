//! Helper input validation. Std only, no `crate::` imports.
//! Anything odd is rejected, never "fixed up".

pub(crate) const MAX_ATOM_LEN: usize = 256;
pub(crate) const MAX_LINE_LEN: usize = 1024;
pub(crate) const MAX_BATCH: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reject {
    Empty,
    TooLong,
    TooMany,
    BadChar,
    BadShape,
}

/// Parsed `[repo/]name`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Atom<'a> {
    pub(crate) repo: Option<&'a str>,
    pub(crate) name: &'a str,
}

/// Files the helper may write. Client sends an id, never a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    World,
    ResumeState,
    LastActionState,
    EmergeLog,
}

impl Target {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Target::World => "/etc/portage/world",
            Target::ResumeState => "/etc/portage/resume.state",
            Target::LastActionState => "/etc/portage/lastaction.state",
            Target::EmergeLog => "/var/log/emerge.log",
        }
    }

    /// Wire id, inverse of `from_id`.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Target::World => "world",
            Target::ResumeState => "resume",
            Target::LastActionState => "lastaction",
            Target::EmergeLog => "log",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id {
            "world" => Some(Target::World),
            "resume" => Some(Target::ResumeState),
            "lastaction" => Some(Target::LastActionState),
            "log" => Some(Target::EmergeLog),
            _ => None,
        }
    }
}

// ASCII only, stricter than mask.rs on purpose (homoglyphs).
fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "@._+-".contains(c)
}

fn is_bidi_or_zw(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200B}'..='\u{200D}'
            | '\u{FEFF}'
    )
}

/// One `[repo/]name` atom.
pub(crate) fn atom(s: &str) -> Result<Atom<'_>, Reject> {
    if s.is_empty() {
        return Err(Reject::Empty);
    }
    if s.len() > MAX_ATOM_LEN {
        return Err(Reject::TooLong);
    }
    let (repo, name) = match s.split_once('/') {
        Some((r, n)) => (Some(r), n),
        None => (None, s),
    };
    if name.is_empty() || repo == Some("") {
        return Err(Reject::BadShape);
    }
    // No leading '-' (option injection) or '.' (hidden / path-like).
    for part in repo.into_iter().chain(std::iter::once(name)) {
        if part.starts_with('-') || part.starts_with('.') {
            return Err(Reject::BadShape);
        }
        if !part.chars().all(name_char) {
            return Err(Reject::BadChar);
        }
    }
    Ok(Atom { repo, name })
}

/// A whole list of atoms, count-limited.
pub(crate) fn atoms(list: &[String]) -> Result<Vec<Atom<'_>>, Reject> {
    if list.is_empty() {
        return Err(Reject::Empty);
    }
    if list.len() > MAX_BATCH {
        return Err(Reject::TooMany);
    }
    list.iter().map(|s| atom(s)).collect()
}

/// Free-form line for world / resume.state / emerge.log.
/// One physical line: no controls (\n, \t, ESC...), no bidi.
pub(crate) fn line(s: &str) -> Result<&str, Reject> {
    if s.is_empty() {
        return Err(Reject::Empty);
    }
    if s.len() > MAX_LINE_LEN {
        return Err(Reject::TooLong);
    }
    if s.chars().any(|c| c.is_control() || is_bidi_or_zw(c)) {
        return Err(Reject::BadChar);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_ok() {
        assert_eq!(
            atom("extra/nano"),
            Ok(Atom {
                repo: Some("extra"),
                name: "nano"
            })
        );
        assert_eq!(
            atom("g++"),
            Ok(Atom {
                repo: None,
                name: "g++"
            })
        );
        assert!(atom("python-zope.interface").is_ok());
    }

    #[test]
    fn atom_rejects_injection() {
        assert_eq!(atom(""), Err(Reject::Empty));
        assert_eq!(atom("--noconfirm"), Err(Reject::BadShape));
        assert_eq!(atom("../etc/passwd"), Err(Reject::BadShape));
        assert_eq!(atom("a/b/c"), Err(Reject::BadChar));
        assert_eq!(atom("nano\nvim"), Err(Reject::BadChar));
        assert_eq!(atom("nano vim"), Err(Reject::BadChar));
        assert_eq!(atom("nano;rm"), Err(Reject::BadChar));
        assert_eq!(atom("/nano"), Err(Reject::BadShape));
        assert_eq!(atom("extra/"), Err(Reject::BadShape));
        assert_eq!(atom("n\u{0430}no"), Err(Reject::BadChar)); // cyrillic 'а'
    }

    #[test]
    fn atom_length_limit() {
        assert_eq!(atom(&"a".repeat(MAX_ATOM_LEN + 1)), Err(Reject::TooLong));
        assert!(atom(&"a".repeat(MAX_ATOM_LEN)).is_ok());
    }

    #[test]
    fn atoms_limits() {
        assert_eq!(atoms(&[]), Err(Reject::Empty));
        let many = vec!["a".to_string(); MAX_BATCH + 1];
        assert_eq!(atoms(&many), Err(Reject::TooMany));
        assert_eq!(
            atoms(&["ok".to_string(), "bad name".to_string()]),
            Err(Reject::BadChar)
        );
    }

    #[test]
    fn line_rejects_controls_and_bidi() {
        assert!(line("2026-09-21 MERGE aur foo-1-1  (3s)").is_ok());
        assert_eq!(line("a\nb"), Err(Reject::BadChar));
        assert_eq!(line("a\x1b[2J"), Err(Reject::BadChar));
        assert_eq!(line("a\u{202E}b"), Err(Reject::BadChar));
        assert_eq!(line(&"x".repeat(MAX_LINE_LEN + 1)), Err(Reject::TooLong));
    }

    #[test]
    fn target_is_id_not_path() {
        assert_eq!(Target::from_id("world"), Some(Target::World));
        assert_eq!(Target::from_id("/etc/shadow"), None);
        assert_eq!(Target::EmergeLog.path(), "/var/log/emerge.log");
        for t in [
            Target::World,
            Target::ResumeState,
            Target::LastActionState,
            Target::EmergeLog,
        ] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
    }
}
