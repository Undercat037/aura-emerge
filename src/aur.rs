//! Direct AUR interaction: git-clones a package's AUR repo and reads its
//! `.SRCINFO` for dependencies, replacing the `aura -A` build/install
//! path (see `packages::aur_install` / `resolve_and_build_aur` for the
//! orchestration that uses this).
//!
//! Deliberately talks to the *official* AUR (`aur.archlinux.org`) for
//! both the git clone and the RPC lookup, rather than the third-party
//! `faur.fosskers.ca` mirror `aura` itself uses -- one fewer trusted
//! party in the chain for a tool whose whole point is being
//! security-conscious about what runs during a build.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const AUR_BASE_URL: &str = "https://aur.archlinux.org";
const AUR_RPC_URL: &str = "https://aur.archlinux.org/rpc/v5/info";
const CURL_BIN: &str = "/usr/bin/curl";
const GIT_BIN: &str = "/usr/bin/git";

/// Shallow-clones `<AUR_BASE_URL>/<pkgbase>.git` into `dest_root/<pkgbase>`.
/// Returns the clone path on success, `None` on any git failure (network,
/// or the name genuinely isn't a pkgbase -- the two aren't distinguished
/// here since callers fall back to an RPC lookup either way).
pub(crate) fn clone_repo(pkgbase: &str, dest_root: &Path) -> Option<PathBuf> {
    let url = format!("{}/{}.git", AUR_BASE_URL, pkgbase);
    let target = dest_root.join(pkgbase);
    let ok = Command::new(GIT_BIN)
        .args(["clone", "--depth=1", "--quiet", &url])
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(target)
}

/// Looks up a package name's real `pkgbase` via the official AUR RPC --
/// only needed for split packages, where the requested name (e.g.
/// `gcc6-libs`) isn't the git repo/branch name (`gcc6`). `None` on any
/// network/parse failure or if the name doesn't exist on the AUR at all.
pub(crate) fn lookup_pkgbase(pkg: &str) -> Option<String> {
    let url = format!("{}?v=5&type=info&arg[]={}", AUR_RPC_URL, urlencode(pkg));
    let out = Command::new(CURL_BIN)
        .args(["-sfL", "--max-time", "10", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&out.stdout);
    extract_json_string_field(&body, "PackageBase")
}

/// Resolves a requested package name to (clone directory, real pkgbase).
/// Tries a direct clone by the given name first -- the common case, since
/// most AUR packages aren't split and the name given usually already is
/// the pkgbase -- and only falls back to an RPC round-trip if that fails,
/// to look up the real pkgbase for a split-package child name.
pub(crate) fn clone_or_resolve(pkg: &str, dest_root: &Path) -> Option<(PathBuf, String)> {
    if let Some(dir) = clone_repo(pkg, dest_root) {
        return Some((dir, pkg.to_string()));
    }
    let pkgbase = lookup_pkgbase(pkg)?;
    if pkgbase == pkg {
        // Already tried this exact name above and it failed -- this is a
        // real clone failure (network, repo gone), not a split-package
        // name mismatch, so retrying it won't help.
        return None;
    }
    let dir = clone_repo(&pkgbase, dest_root)?;
    Some((dir, pkgbase))
}

/// Minimal `.SRCINFO` reader: sums `depends`/`makedepends`/`checkdepends`
/// (including `_<arch>`-suffixed variants) across every section in the
/// file, strips version constraints (`foo>=1.2` -> `foo`), and dedupes.
///
/// `.SRCINFO` is a static, checked-in, generated file (`makepkg
/// --printsrcinfo`) -- reading it never executes anything, unlike parsing
/// `PKGBUILD` itself. Returns `None` only if the file can't be read at
/// all (missing `.SRCINFO` is rare but not unheard of in a malformed/very
/// old AUR repo) -- an empty-but-present file correctly yields `Some(vec![])`.
pub(crate) fn srcinfo_dependencies(path: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else { continue };
        let key = key.trim();
        let value = value.trim();
        let base_key = key.split('_').next().unwrap_or(key);
        if !matches!(base_key, "depends" | "makedepends" | "checkdepends") {
            continue;
        }
        let bare = value.split(['<', '>', '=']).next().unwrap_or(value).trim();
        if !bare.is_empty() {
            out.push(bare.to_string());
        }
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// The `pkgbase = ...` value declared in a `.SRCINFO` file, if present.
pub(crate) fn srcinfo_pkgbase(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let (key, value) = l.trim().split_once('=')?;
        (key.trim() == "pkgbase").then(|| value.trim().to_string())
    })
}

fn urlencode(s: &str) -> String {
    // AUR package names are restricted to alnum + a small punctuation set
    // (-, _, ., @, +) -- none of that needs escaping in a query string, so
    // this is deliberately minimal rather than a general percent-encoder.
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '+') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

/// Extracts a single `"Field":"value"` string from a flat JSON blob --
/// enough for the one field (`PackageBase`) this module needs out of the
/// AUR RPC response, without pulling in a JSON dependency for it. Bails
/// to `None` (rather than guessing) if the field isn't found as a plain
/// string value.
fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\":\"", field);
    let start = json.find(&needle)? + needle.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srcinfo_dependencies_sums_strips_versions_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("aur-srcinfo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".SRCINFO");
        std::fs::write(
            &path,
            "pkgbase = foo\n\tpkgver = 1.0\n\tmakedepends = cmake\n\tdepends = glibc>=2.38\n\npkgname = foo\n\tdepends = zlib\n\tdepends = glibc\n",
        )
        .unwrap();
        let deps = srcinfo_dependencies(&path).unwrap();
        assert_eq!(deps, vec!["cmake", "glibc", "zlib"]);
        assert_eq!(srcinfo_pkgbase(&path), Some("foo".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn srcinfo_dependencies_empty_file_is_some_empty() {
        let dir = std::env::temp_dir().join(format!("aur-srcinfo-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".SRCINFO");
        std::fs::write(&path, "pkgbase = bare\n\tpkgver = 1.0\n").unwrap();
        assert_eq!(srcinfo_dependencies(&path), Some(Vec::new()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn srcinfo_dependencies_missing_file_is_none() {
        let path = Path::new("/nonexistent/definitely/.SRCINFO");
        assert_eq!(srcinfo_dependencies(path), None);
    }

    #[test]
    fn extract_json_string_field_finds_value() {
        let json = r#"{"results":[{"Name":"foo","PackageBase":"foo-base"}]}"#;
        assert_eq!(extract_json_string_field(json, "PackageBase"), Some("foo-base".to_string()));
    }

    #[test]
    fn extract_json_string_field_missing_is_none() {
        let json = r#"{"results":[]}"#;
        assert_eq!(extract_json_string_field(json, "PackageBase"), None);
    }
}