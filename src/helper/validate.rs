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

/// How `remove` treats dependencies and configs. The client picks one
/// of these, never raw flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoveMode {
    /// `-R`: refuses if something still needs the package.
    Plain,
    /// `-Rdd --nosave` (`emerge -C`): unconditional, no .pacsave.
    Unmerge,
    /// `-Rns` (prune/depclean): takes unneeded deps too, no .pacsave.
    Prune,
}

impl RemoveMode {
    /// Wire verb, inverse of `from_id`.
    pub(crate) fn id(self) -> &'static str {
        match self {
            RemoveMode::Plain => "remove",
            RemoveMode::Unmerge => "unmerge",
            RemoveMode::Prune => "prune",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id {
            "remove" => Some(RemoveMode::Plain),
            "unmerge" => Some(RemoveMode::Unmerge),
            "prune" => Some(RemoveMode::Prune),
            _ => None,
        }
    }
}

/// Root of the local package repositories (aurutils-style), one
/// sub-directory per `LocalRepo`. Root-owned, never user-writable.
pub(crate) const LOCAL_REPO_ROOT: &str = "/var/lib/aura-emerge/repos";

/// The local repositories the helper maintains. The client picks one by
/// id, never a path. Names carry an `ae-` prefix so they can never
/// collide with a repo the user already has in pacman.conf (aurutils'
/// own `[aur]` / `[custom]`, say); a bare `local` is reserved by libalpm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalRepo {
    /// Packages built from the AUR.
    Aur,
    /// Packages built from ABS (Arch build system) trees.
    Abs,
    /// Packages built from a local PKGBUILD or installed from archives.
    Local,
}

impl LocalRepo {
    pub(crate) const ALL: [LocalRepo; 3] = [LocalRepo::Aur, LocalRepo::Abs, LocalRepo::Local];

    /// libalpm db name / pacman.conf section / wire id.
    pub(crate) fn name(self) -> &'static str {
        match self {
            LocalRepo::Aur => "ae-aur",
            LocalRepo::Abs => "ae-abs",
            LocalRepo::Local => "ae-local",
        }
    }

    /// World / user-facing source prefix. The alpm db stays `ae-*`
    /// (no clash with user repos); world keeps `aur/` / `abs/` / `local/`
    /// so a rebuild still means the original origin, not "install from
    /// the local binary repo".
    pub(crate) fn source_prefix(self) -> &'static str {
        match self {
            LocalRepo::Aur => "aur",
            LocalRepo::Abs => "abs",
            LocalRepo::Local => "local",
        }
    }

    /// Wire id, inverse of `from_id`.
    pub(crate) fn id(self) -> &'static str {
        self.name()
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == id)
    }

    /// Directory that holds the packages and the db (`<name>.db`).
    pub(crate) fn dir(self) -> &'static str {
        match self {
            LocalRepo::Aur => "/var/lib/aura-emerge/repos/ae-aur",
            LocalRepo::Abs => "/var/lib/aura-emerge/repos/ae-abs",
            LocalRepo::Local => "/var/lib/aura-emerge/repos/ae-local",
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

/// Suffixes AE Force leaves next to a file (also as `<ext>.<n>`).
pub(crate) const AE_FORCE_EXT: &str = ".aeforce";
pub(crate) const AE_NEW_EXT: &str = ".aenew";
pub(crate) const AE_PARK_EXT: &str = ".aepark";
pub(crate) const AE_EXTS: [&str; 3] = [AE_FORCE_EXT, AE_NEW_EXT, AE_PARK_EXT];
/// Highest `<ext>.<n>` tried when `<ext>` is already taken.
pub(crate) const AE_MAX_N: u32 = 99;

/// `-U` switches the client may pick (never raw flags).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FileOpts {
    /// `--needed`: skip what is already installed at that version or newer.
    pub(crate) needed: bool,
    /// `--asdeps`: mark the targets as dependencies.
    pub(crate) asdeps: bool,
    /// Auto-remove installed packages that conflict with this `-U`
    /// (only after the frontend asked / `--noconfirm`).
    pub(crate) replace: bool,
    /// Move unowned files that block the install to `<file>.aeforce`.
    pub(crate) aeforce: bool,
    /// Keep old unowned files, install the new ones as `<file>.aenew`.
    pub(crate) aenew: bool,
    /// `--noaefile`: nothing is kept aside. Alone: old files stay, new
    /// ones are dropped. With `aeforce`: new files win, old ones are dropped.
    pub(crate) aekeep: bool,
}

impl FileOpts {
    /// Wire form, inverse of `from_id` (comma-separated flags, or `-`).
    pub(crate) fn id(self) -> String {
        let mut parts = Vec::new();
        if self.needed {
            parts.push("needed");
        }
        if self.asdeps {
            parts.push("asdeps");
        }
        if self.replace {
            parts.push("replace");
        }
        if self.aeforce {
            parts.push("aeforce");
        }
        if self.aenew {
            parts.push("aenew");
        }
        if self.aekeep {
            parts.push("aekeep");
        }
        if parts.is_empty() {
            "-".to_string()
        } else {
            parts.join(",")
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        if id == "-" {
            return Some(FileOpts::default());
        }
        let mut needed = false;
        let mut asdeps = false;
        let mut replace = false;
        let mut aeforce = false;
        let mut aenew = false;
        let mut aekeep = false;
        for part in id.split(',') {
            match part {
                "needed" if !needed => needed = true,
                "asdeps" if !asdeps => asdeps = true,
                "replace" if !replace => replace = true,
                "aeforce" if !aeforce => aeforce = true,
                "aenew" if !aenew => aenew = true,
                "aekeep" if !aekeep => aekeep = true,
                _ => return None,
            }
        }
        Some(FileOpts {
            needed,
            asdeps,
            replace,
            aeforce,
            aenew,
            aekeep,
        })
    }
}

/// `<sha256> <abs path>` for `-U`: what the client says it audited.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileSpec<'a> {
    pub(crate) sha256: &'a str,
    pub(crate) path: &'a str,
}

/// Lowercase hex only, exactly 64 chars.
fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn file_spec(s: &str) -> Result<FileSpec<'_>, Reject> {
    let s = line(s)?;
    let (sha256, path) = s.split_once(' ').ok_or(Reject::BadShape)?;
    if !is_sha256(sha256) {
        return Err(Reject::BadShape);
    }
    // Absolute, no `..`, not a dir-looking path; the client canonicalizes.
    if !path.starts_with('/') || path.ends_with('/') || path.split('/').any(|c| c == "..") {
        return Err(Reject::BadShape);
    }
    Ok(FileSpec { sha256, path })
}

pub(crate) fn file_specs(list: &[String]) -> Result<Vec<FileSpec<'_>>, Reject> {
    if list.is_empty() {
        return Err(Reject::Empty);
    }
    if list.len() > MAX_BATCH {
        return Err(Reject::TooMany);
    }
    list.iter().map(|s| file_spec(s)).collect()
}

/// Client side: builds a spec line (validated the same way).
pub(crate) fn spec(sha256: &str, path: &str) -> Result<String, Reject> {
    let s = format!("{} {}", sha256, path);
    file_spec(&s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn file_spec_ok() {
        let s = format!("{} /home/u/my pkg-1-1-any.pkg.tar.zst", SHA);
        let f = file_spec(&s).unwrap();
        assert_eq!(f.sha256, SHA);
        assert_eq!(f.path, "/home/u/my pkg-1-1-any.pkg.tar.zst");
    }

    #[test]
    fn file_spec_rejects_junk() {
        let up = SHA.to_uppercase();
        for bad in [
            "".to_string(),
            SHA.to_string(),                         // no path
            format!("{} rel/path.pkg", SHA),         // relative
            format!("{} /a/../etc/x", SHA),          // dotdot
            format!("{} /a/dir/", SHA),              // dir
            format!("{} /a\nb", SHA),                // newline
            format!("{} /a\tb", SHA),                // tab
            format!("{}0 /a", SHA),                  // 65 hex
            format!("{} /a", &SHA[..63]),            // 63 hex
            format!("{} /a", up),                    // uppercase
            format!("{} /a", SHA.replace('b', "g")), // non-hex
        ] {
            assert!(file_spec(&bad).is_err(), "{:?}", bad);
        }
        assert_eq!(file_specs(&[]), Err(Reject::Empty));
    }

    #[test]
    fn file_opts_ids_roundtrip_and_reject_junk() {
        for (n, d, r) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            let o = FileOpts {
                needed: n,
                asdeps: d,
                replace: r,
                aeforce: n && r,
                aenew: d && r,
                aekeep: n && !d,
            };
            assert_eq!(FileOpts::from_id(&o.id()), Some(o));
        }
        // Order of flags does not matter.
        assert_eq!(
            FileOpts::from_id("asdeps,needed"),
            Some(FileOpts {
                needed: true,
                asdeps: true,
                replace: false,
                aeforce: false,
                aenew: false,
                aekeep: false,
            })
        );
        assert_eq!(
            FileOpts::from_id("aekeep"),
            Some(FileOpts {
                aekeep: true,
                ..FileOpts::default()
            })
        );
        assert_eq!(
            FileOpts::from_id("aenew"),
            Some(FileOpts {
                aenew: true,
                ..FileOpts::default()
            })
        );
        assert_eq!(
            FileOpts::from_id("aeforce"),
            Some(FileOpts {
                aeforce: true,
                ..FileOpts::default()
            })
        );
        for bad in [
            "",
            "--needed",
            "needed,",
            "NEEDED",
            "needed asdeps",
            "needed,needed",
            "replace,bogus",
        ] {
            assert_eq!(FileOpts::from_id(bad), None, "{:?}", bad);
        }
    }

    #[test]
    fn spec_roundtrips() {
        let s = spec(SHA, "/tmp/a.pkg.tar.zst").unwrap();
        assert_eq!(file_spec(&s).unwrap().path, "/tmp/a.pkg.tar.zst");
        assert!(spec(SHA, "relative").is_err());
    }

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

    #[test]
    fn local_repo_ids_roundtrip_and_stay_under_the_root() {
        for r in LocalRepo::ALL {
            assert_eq!(LocalRepo::from_id(r.id()), Some(r));
            assert_eq!(r.dir(), format!("{}/{}", LOCAL_REPO_ROOT, r.name()));
            assert!(atom(r.name()).is_ok());
        }
        for bad in [
            "",
            "local",
            "aur",
            "ae-aur/",
            "../ae-aur",
            "ae-AUR",
            "ae-aur ",
        ] {
            assert_eq!(LocalRepo::from_id(bad), None, "{:?}", bad);
        }
    }
}
