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
const AUR_RPC_INFO_URL: &str = "https://aur.archlinux.org/rpc/v5/info";
const AUR_RPC_SEARCH_URL: &str = "https://aur.archlinux.org/rpc/v5/search";
pub(crate) const GIT_BIN: &str = "/usr/bin/git";
/// AUR RPC caps `arg[]=` batches; stay comfortably under it.
const RPC_INFO_BATCH: usize = 100;

/// Subset of the AUR RPC v5 `info`/`search` response fields this tool
/// actually uses -- replaces parsing `aura -Ai`/`aura -As` stdout.
#[derive(Debug, Clone)]
pub(crate) struct AurPkgInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) pkgbase: String,
    pub(crate) num_votes: i64,
    pub(crate) popularity: f64,
    /// `OutOfDate` is `null` or a unix timestamp in the RPC response --
    /// collapsed to a bool here, callers don't need the flag date.
    pub(crate) out_of_date: bool,
    pub(crate) maintainer: Option<String>,
}

/// Shallow-clones `<AUR_BASE_URL>/<pkgbase>.git` into `dest_root/<pkgbase>`.
/// Returns the clone path on success, `None` on any git failure (network,
/// or the name genuinely isn't a pkgbase -- the two aren't distinguished
/// here since callers fall back to an RPC lookup either way).
///
/// `dest_root` is expected to be a directory `clone_repo`'s caller owns
/// exclusively for this run (see `packages::aur_build_base()`/
/// `abs_build_base()`'s wipe-on-start) -- if `target` already exists, that's a
/// caller-side bug (stale wipe, or the same pkgbase cloned twice in one
/// run), not a normal "package doesn't exist" case. Reported distinctly
/// on stderr so it doesn't masquerade as an AUR miss several call-frames
/// up in `clone_or_resolve()`/`resolve_and_build_aur()`.
pub(crate) fn clone_repo(pkgbase: &str, dest_root: &Path) -> Option<PathBuf> {
    let url = format!("{}/{}.git", AUR_BASE_URL, pkgbase);
    let target = dest_root.join(pkgbase);
    if target.exists() {
        eprintln!(
            ">>> Warning: build target {} already exists (stale from an earlier clone in this run?) -- skipping clone",
            target.display()
        );
        return None;
    }
    let output = Command::new(GIT_BIN)
        .args(["clone", "--depth=1", "--quiet", &url])
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let ok = match &output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    };
    if !ok {
        // Surface git's own stderr at a low volume rather than
        // discarding it -- a genuine "repository not found" (bad/unknown
        // pkgbase, the expected/common case) looks very different from a
        // network timeout or TLS failure, and telling those apart used
        // to require re-running with `git clone` by hand.
        if let Ok(o) = &output {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stderr = stderr.trim();
            if !stderr.is_empty() && !stderr.contains("Repository not found") {
                eprintln!(">>> Note: git clone of '{}' failed: {}", pkgbase, stderr);
            }
        }
        return None;
    }
    // AUR's git backend happily "clones" successfully (exit 0) for a name
    // that was never actually pushed as a package -- it just hands back an
    // empty, zero-commit repo with no PKGBUILD in it. Treat that the same
    // as "package doesn't exist" rather than letting the bogus empty clone
    // limp downstream into a much more confusing failure several layers
    // later (missing .SRCINFO, then "could not read PKGBUILD: No such
    // file or directory"). Clean up the empty dir too, since dest_root is
    // a fixed, reused-across-runs path (packages::aur_build_base()) -- leaving it
    // behind would otherwise sit there as a stale, empty trap for the
    // next run.
    if !target.join("PKGBUILD").is_file() {
        let _ = std::fs::remove_dir_all(&target);
        return None;
    }
    Some(target)
}

/// Looks up a package name's real `pkgbase` via the official AUR RPC --
/// only needed for split packages, where the requested name (e.g.
/// `gcc6-libs`) isn't the git repo/branch name (`gcc6`). `None` on any
/// network/parse failure or if the name doesn't exist on the AUR at all.
pub(crate) fn lookup_pkgbase(pkg: &str) -> Option<String> {
    let url = format!("{}?v=5&type=info&arg[]={}", AUR_RPC_INFO_URL, urlencode(pkg));
    let body = crate::http::get(&url, 10)?;
    extract_json_string_field(&body, "PackageBase")
}

/// Batched AUR RPC `info` lookup -- replaces parsing `aura -Ai <pkg>`
/// stdout one package at a time. Chunks the request at `RPC_INFO_BATCH`
/// names per call (the RPC caps how many `arg[]=` params it'll accept),
/// so this is at most a handful of round-trips even for a large
/// `@world` upgrade. Silently skips a chunk that fails (network hiccup,
/// timeout) rather than failing the whole batch -- callers treat a name
/// missing from the result as "not found"/"skip", same as an aura -Ai
/// miss did before.
pub(crate) fn rpc_info(names: &[String]) -> Vec<AurPkgInfo> {
    let mut out = Vec::new();
    for chunk in names.chunks(RPC_INFO_BATCH) {
        let mut url = format!("{}?v=5&type=info", AUR_RPC_INFO_URL);
        for n in chunk {
            url.push_str("&arg[]=");
            url.push_str(&urlencode(n));
        }
        let Some(body) = http_get(&url) else { continue };
        out.extend(parse_pkg_results(&body));
    }
    out
}

/// AUR RPC `search`, either by package name (`by_desc = false`) or by
/// name+description (`by_desc = true`) -- replaces `aura -As`/`aura -Ss`
/// (AUR half) and `aura --searchdesc` (AUR half) output parsing.
/// Empty/no-match results in an empty Vec, same as a real miss; `None`
/// is never returned since an empty result list already communicates
/// "no results" to callers, and there's nothing more specific a caller
/// would do differently for a network failure vs a genuine zero-match.
pub(crate) fn rpc_search(term: &str, by_desc: bool) -> Vec<AurPkgInfo> {
    let by = if by_desc { "name-desc" } else { "name" };
    let url = format!("{}/{}?v=5&by={}", AUR_RPC_SEARCH_URL, urlencode(term), by);
    match http_get(&url) {
        Some(body) => parse_pkg_results(&body),
        None => Vec::new(),
    }
}

fn http_get(url: &str) -> Option<String> {
    crate::http::get(url, 10)
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
/// -- both the bare key and its `_<current_arch>`-suffixed variant (e.g.
/// `depends_x86_64` when `current_arch` is `"x86_64"`) -- across every
/// section in the file, strips version constraints (`foo>=1.2` -> `foo`),
/// and dedupes. A different arch's suffix (`depends_aarch64` on an
/// x86_64 machine) is deliberately excluded -- summing every arch
/// unconditionally would hand `pacman -S` package names that don't even
/// exist for this machine.
///
/// `.SRCINFO` is a static, checked-in, generated file (`makepkg
/// --printsrcinfo`) -- reading it never executes anything, unlike parsing
/// `PKGBUILD` itself. Returns `None` only if the file can't be read at
/// all (missing `.SRCINFO` is rare but not unheard of in a malformed/very
/// old AUR repo) -- an empty-but-present file correctly yields `Some(vec![])`.
pub(crate) fn srcinfo_dependencies(path: &Path, current_arch: &str) -> Option<Vec<String>> {
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
        // Either the bare key, or arch-suffixed for exactly this machine's
        // arch -- e.g. accept "depends" and "depends_x86_64" when
        // current_arch == "x86_64", but skip "depends_aarch64".
        if key != base_key && key != format!("{base_key}_{current_arch}") {
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

/// Parses the `"results":[...]` array of an AUR RPC `info`/`search`
/// response into `AurPkgInfo`s. Skips any object missing a `Name` (there
/// shouldn't be one, but this is minimal hand-rolled parsing, not a real
/// JSON parser -- bailing per-object on a shape surprise beats guessing).
fn parse_pkg_results(json: &str) -> Vec<AurPkgInfo> {
    let Some(array_body) = extract_results_array(json) else { return Vec::new() };
    split_json_objects(array_body)
        .into_iter()
        .filter_map(|obj| {
            let name = extract_json_string_field(obj, "Name")?;
            let version = extract_json_string_field(obj, "Version").unwrap_or_default();
            let description = extract_json_string_field(obj, "Description").unwrap_or_default();
            let pkgbase = extract_json_string_field(obj, "PackageBase").unwrap_or_else(|| name.clone());
            let num_votes = extract_json_number_field(obj, "NumVotes").unwrap_or(0.0) as i64;
            let popularity = extract_json_number_field(obj, "Popularity").unwrap_or(0.0);
            let out_of_date = extract_json_raw_field(obj, "OutOfDate")
                .map(|v| v.trim() != "null")
                .unwrap_or(false);
            let maintainer = extract_json_string_field(obj, "Maintainer");
            Some(AurPkgInfo { name, version, description, pkgbase, num_votes, popularity, out_of_date, maintainer })
        })
        .collect()
}

/// Slices out the contents between the `[` and matching `]` of the
/// top-level `"results":` array, ignoring brackets inside string values.
fn extract_results_array(json: &str) -> Option<&str> {
    let key = "\"results\":";
    let start = json.find(key)? + key.len();
    let rest = &json[start..];
    let open = rest.find('[')?;
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        let c = b as char;
        if in_string {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if c == '"' { in_string = false; }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[open + 1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits a JSON array's inner content (as returned by
/// `extract_results_array`) into its top-level `{...}` object substrings,
/// respecting nested braces and braces inside strings.
fn split_json_objects(array_body: &str) -> Vec<&str> {
    let bytes = array_body.as_bytes();
    let mut objs = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        if in_string {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if c == '"' { in_string = false; }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => {
                if depth == 0 { start = i; }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    objs.push(&array_body[start..=i]);
                }
            }
            _ => {}
        }
    }
    objs
}

/// Extracts a bare (unquoted) numeric field value, e.g. `"NumVotes":42`.
fn extract_json_number_field(json: &str, field: &str) -> Option<f64> {
    let needle = format!("\"{}\":", field);
    let start = json.find(&needle)? + needle.len();
    let tail = json[start..].trim_start();
    let end = tail.find([',', '}']).unwrap_or(tail.len());
    tail[..end].trim().parse().ok()
}

/// Extracts a field's raw (un-typed) value slice, e.g. `null` or `1700000000`
/// for `OutOfDate` -- enough to distinguish "null" from "set" without
/// needing to know in advance whether it's a number or a literal.
fn extract_json_raw_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\":", field);
    let start = json.find(&needle)? + needle.len();
    let tail = &json[start..];
    let end = tail.find([',', '}']).unwrap_or(tail.len());
    Some(tail[..end].trim().to_string())
}

/// Extracts a single `"Field":"value"` string from a flat JSON blob --
/// enough for the handful of string fields (`PackageBase`, `Name`,
/// `Description`, ...) this module needs out of the AUR RPC response,
/// without pulling in a JSON dependency for it. Honors `\"`/`\\` escapes
/// while scanning for the closing quote (needed for `Description`, which
/// routinely contains apostrophes/quotes) and decodes the small set of
/// escape sequences the AUR RPC actually emits. `null` (used for e.g. an
/// absent `Maintainer`) is treated as absent, same as a missing field.
/// Bails to `None` if the field isn't present as a string value.
fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\":", field);
    let start = json.find(&needle)? + needle.len();
    let tail = json[start..].trim_start();
    if tail.starts_with("null") {
        return None;
    }
    let tail = tail.strip_prefix('"')?;
    let bytes = tail.as_bytes();
    let mut end = None;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        if escaped { escaped = false; continue; }
        match c {
            '\\' => escaped = true,
            '"' => { end = Some(i); break; }
            _ => {}
        }
    }
    let raw = &tail[..end?];
    Some(decode_entities_json(raw))
}

/// Decodes the JSON string escapes the AUR RPC actually emits
/// (`\"`, `\\`, `\/`, `\n`, `\t`, `\uXXXX`) -- not a general JSON-string
/// decoder, just enough for package names/descriptions/URLs.
fn decode_entities_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(cp) {
                        out.push(ch);
                    }
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
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
        let deps = srcinfo_dependencies(&path, "x86_64").unwrap();
        assert_eq!(deps, vec!["cmake", "glibc", "zlib"]);
        assert_eq!(srcinfo_pkgbase(&path), Some("foo".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn srcinfo_dependencies_includes_current_arch_suffix_only() {
        let dir = std::env::temp_dir().join(format!("aur-srcinfo-arch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".SRCINFO");
        std::fs::write(
            &path,
            "pkgbase = foo\n\tdepends = glibc\n\tdepends_x86_64 = lib32-glibc\n\tdepends_aarch64 = some-aarch64-only-lib\n",
        )
        .unwrap();
        let deps = srcinfo_dependencies(&path, "x86_64").unwrap();
        assert_eq!(deps, vec!["glibc", "lib32-glibc"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn srcinfo_dependencies_empty_file_is_some_empty() {
        let dir = std::env::temp_dir().join(format!("aur-srcinfo-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".SRCINFO");
        std::fs::write(&path, "pkgbase = bare\n\tpkgver = 1.0\n").unwrap();
        assert_eq!(srcinfo_dependencies(&path, "x86_64"), Some(Vec::new()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn srcinfo_dependencies_missing_file_is_none() {
        let path = Path::new("/nonexistent/definitely/.SRCINFO");
        assert_eq!(srcinfo_dependencies(path, "x86_64"), None);
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

    #[test]
    fn extract_json_string_field_handles_escaped_quotes() {
        let json = r#"{"Description":"A \"quoted\" word, and a slash \/ too"}"#;
        assert_eq!(
            extract_json_string_field(json, "Description"),
            Some("A \"quoted\" word, and a slash / too".to_string())
        );
    }

    #[test]
    fn extract_json_string_field_null_is_none() {
        let json = r#"{"Maintainer":null}"#;
        assert_eq!(extract_json_string_field(json, "Maintainer"), None);
    }

    #[test]
    fn parse_pkg_results_multiple_objects() {
        let json = r#"{"resultcount":2,"results":[
            {"Name":"foo","Version":"1.0-1","PackageBase":"foo","Description":"the foo pkg","NumVotes":10,"Popularity":1.5,"OutOfDate":null,"Maintainer":"alice"},
            {"Name":"bar","Version":"2.0-1","PackageBase":"bar-base","Description":"has a \"quote\"","NumVotes":0,"Popularity":0.0,"OutOfDate":1700000000,"Maintainer":null}
        ],"type":"search"}"#;
        let results = parse_pkg_results(json);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "foo");
        assert_eq!(results[0].version, "1.0-1");
        assert_eq!(results[0].num_votes, 10);
        assert!(!results[0].out_of_date);
        assert_eq!(results[0].maintainer, Some("alice".to_string()));
        assert_eq!(results[1].pkgbase, "bar-base");
        assert_eq!(results[1].description, "has a \"quote\"");
        assert!(results[1].out_of_date);
        assert_eq!(results[1].maintainer, None);
    }

    #[test]
    fn parse_pkg_results_empty_array() {
        let json = r#"{"resultcount":0,"results":[],"type":"search"}"#;
        assert!(parse_pkg_results(json).is_empty());
    }
}