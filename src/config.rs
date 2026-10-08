//! `/etc/portage/make.conf` only (system path; no per-user override).
//!
//! Two jobs in one file, like real Portage's `make.conf`:
//!   * `EMERGE_DEFAULT_OPTS` -- flags spliced into argv before clap
//!     sees it. `--ignore-default-opts` skips them for one run.
//!   * `CFLAGS`/`CXXFLAGS`/`LDFLAGS`/`RUSTFLAGS`/`MAKEFLAGS`/
//!     `NINJAFLAGS`/`BUILDENV`/`OPTIONS`, applied via a generated
//!     makepkg.conf passed to makepkg as `--config`.
//!
//! Flat, bash-assignment syntax -- no `[build]` table, same as real
//! `make.conf`:
//!
//! ```text
//! EMERGE_DEFAULT_OPTS="--pkgbuild-view --unshare-net-build"
//!
//! CFLAGS="-march=native -O2 -pipe"
//! MAKEFLAGS="-j$(nproc)"
//! OPTIONS=(strip !debug)
//! ```
//!
//! `KEY="..."` and bare `KEY=...` both allow `$`/`` ` `` to survive
//! into the generated makepkg.conf for shell expansion at build time
//! (so `-j$(nproc)` works). `KEY='...'` is a literal single-quoted
//! string -- no expansion, ever, even once re-embedded in the
//! generated file. `KEY=(a b c)` is an array; array entries may be
//! bare or quoted. `#` starts a comment outside quotes.
//!
//! Scalars and lists are interchangeable: a list is joined with
//! spaces, a string is split on them.
//!
//! System file first, then the user one; last to set a key wins.
//! Trust note: a value here becomes shell code run as the build user,
//! the same trust level `/etc/makepkg.conf` already has.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use colored::Colorize;

pub(crate) const SYSTEM_CONF: &str = "/etc/portage/make.conf";

/// Build keys understood at the top level.
pub(crate) const BUILD_VARS: &[&str] = &[
    "CFLAGS",
    "CXXFLAGS",
    "CPPFLAGS",
    "LDFLAGS",
    "RUSTFLAGS",
    "MAKEFLAGS",
    "NINJAFLAGS",
    "BUILDENV",
    "OPTIONS",
];

/// Bash-array keys, written as `KEY=(a b c)` not a quoted scalar.
const ARRAY_VARS: &[&str] = &["BUILDENV", "OPTIONS"];

/// Keys makepkg doesn't export itself (NINJAFLAGS is a PKGBUILD
/// convention, not a makepkg variable), so we export them.
const EXPORT_VARS: &[&str] = &["NINJAFLAGS"];

/// Key for spliced-in default flags -- the Portage name, since that's
/// exactly what it is.
const DEFAULT_FLAG_KEY: &str = "EMERGE_DEFAULT_OPTS";

/// Read into `vars` but not a build var: never reaches makepkg.
const FEATURES_KEY: &str = "FEATURES";

/// A build value, kept as written so arrays stay arrays in the
/// generated makepkg.conf.
#[derive(Debug, Clone)]
pub(crate) enum BuildValue {
    /// Bare or double-quoted: `$`/`` ` `` survive for shell expansion
    /// at build time.
    Scalar(String),
    /// Single-quoted: literal, no expansion -- re-escaped if it ends
    /// up back inside a double-quoted string (see `esc_double_literal`).
    LiteralScalar(String),
    List(Vec<String>),
}

impl BuildValue {
    /// One-line form, for `--info` and messages.
    pub(crate) fn display(&self) -> String {
        match self {
            BuildValue::Scalar(s) | BuildValue::LiteralScalar(s) => s.clone(),
            BuildValue::List(v) => v.join(" "),
        }
    }

    /// Tokens, for keys makepkg holds as a bash array.
    fn tokens(&self) -> Vec<String> {
        match self {
            BuildValue::Scalar(s) | BuildValue::LiteralScalar(s) => {
                s.split_whitespace().map(str::to_string).collect()
            }
            BuildValue::List(v) => v.clone(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Config {
    /// `EMERGE_DEFAULT_OPTS`, already one token per entry.
    pub(crate) default_flags: Vec<String>,
    /// `FEATURES` tokens as written (`-foo` turns `foo` off).
    pub(crate) features: Vec<String>,
    /// Build vars actually set, in `BUILD_VARS` order.
    pub(crate) build_vars: Vec<(String, BuildValue)>,
    /// Config files that were read, in precedence order.
    pub(crate) files: Vec<PathBuf>,
}

/// Reads `/etc/portage/make.conf` only. Per-user
/// `~/.config/emerge/make.conf` is intentionally not read (root-owned
/// system config is the single source of truth).
/// Missing file is fine; a symlink is refused, as elsewhere.
pub(crate) fn load() -> Config {
    let mut vars: HashMap<String, BuildValue> = HashMap::new();
    let mut default_flags: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    let paths: Vec<PathBuf> = vec![PathBuf::from(SYSTEM_CONF)];

    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = match crate::read_to_string_nofollow(&path) {
            Ok(t) => t,
            Err(e) if crate::is_symlink_open_error(&e) => {
                eprintln!(
                    "{} {} is a symlink - refusing to read",
                    ">>> Warning:".yellow().bold(),
                    path.display()
                );
                continue;
            }
            Err(_) => {
                eprintln!(
                    "{} could not read {} - ignoring it",
                    ">>> Warning:".yellow().bold(),
                    path.display()
                );
                continue;
            }
        };
        if parse_into(&text, &path, &mut vars, &mut default_flags) {
            files.push(path);
        }
    }

    let build_vars: Vec<(String, BuildValue)> = BUILD_VARS
        .iter()
        .filter_map(|k| vars.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();

    let features = features_of(&vars);

    Config {
        default_flags,
        features,
        build_vars,
        files,
    }
}

fn features_of(vars: &HashMap<String, BuildValue>) -> Vec<String> {
    vars.get(FEATURES_KEY)
        .map(BuildValue::tokens)
        .unwrap_or_default()
}

/// Stores one parsed `key = value` into `vars`/`default_flags`, or
/// warns if `key` isn't recognized.
fn store(
    key: String,
    value: BuildValue,
    path: &Path,
    vars: &mut HashMap<String, BuildValue>,
    default_flags: &mut Vec<String>,
) {
    if key == DEFAULT_FLAG_KEY {
        *default_flags = value.tokens();
    } else if key == FEATURES_KEY || BUILD_VARS.contains(&key.as_str()) {
        vars.insert(key, value);
    } else {
        warn(path, &key, "unknown key (ignored)");
    }
}

/// Parses one make.conf-style file into the accumulators. False if it
/// had a structural error (unterminated quote or array); a bad or
/// unknown key just warns and is skipped.
fn parse_into(
    text: &str,
    path: &Path,
    vars: &mut HashMap<String, BuildValue>,
    default_flags: &mut Vec<String>,
) -> bool {
    let cs: Vec<char> = text.chars().collect();
    let n = cs.len();
    let mut i = 0;
    let mut ok = true;

    while i < n {
        while i < n && cs[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if cs[i] == '#' {
            while i < n && cs[i] != '\n' {
                i += 1;
            }
            continue;
        }

        let key_start = i;
        while i < n && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
            i += 1;
        }
        if i == key_start {
            warn(path, "?", "unexpected character (line skipped)");
            while i < n && cs[i] != '\n' {
                i += 1;
            }
            ok = false;
            continue;
        }
        let key: String = cs[key_start..i].iter().collect();

        while i < n && (cs[i] == ' ' || cs[i] == '\t') {
            i += 1;
        }
        if i >= n || cs[i] != '=' {
            warn(path, &key, "expected '=' after key (line skipped)");
            while i < n && cs[i] != '\n' {
                i += 1;
            }
            ok = false;
            continue;
        }
        i += 1;
        while i < n && (cs[i] == ' ' || cs[i] == '\t') {
            i += 1;
        }

        if i < n && cs[i] == '(' {
            i += 1;
            let mut tokens = Vec::new();
            let mut closed = false;
            while i < n {
                while i < n && cs[i].is_whitespace() {
                    i += 1;
                }
                if i >= n {
                    break;
                }
                if cs[i] == ')' {
                    i += 1;
                    closed = true;
                    break;
                }
                if cs[i] == '#' {
                    while i < n && cs[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                if cs[i] == '"' || cs[i] == '\'' {
                    let q = cs[i];
                    i += 1;
                    let tok_start = i;
                    while i < n && cs[i] != q {
                        i += 1;
                    }
                    tokens.push(cs[tok_start..i].iter().collect());
                    if i < n {
                        i += 1;
                    }
                } else {
                    let tok_start = i;
                    while i < n && !cs[i].is_whitespace() && cs[i] != ')' {
                        i += 1;
                    }
                    tokens.push(cs[tok_start..i].iter().collect());
                }
            }
            if !closed {
                warn(path, &key, "unterminated array (missing ')')");
                ok = false;
            }
            store(key, BuildValue::List(tokens), path, vars, default_flags);
        } else if i < n && (cs[i] == '"' || cs[i] == '\'') {
            let q = cs[i];
            i += 1;
            let val_start = i;
            if q == '"' {
                // `\"` doesn't end the string; anything else (`$`,
                // `` ` ``, other backslashes) passes through raw.
                while i < n && cs[i] != '"' {
                    if cs[i] == '\\' && i + 1 < n {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            } else {
                while i < n && cs[i] != '\'' {
                    i += 1;
                }
            }
            if i >= n {
                warn(path, &key, "unterminated quoted value");
                ok = false;
                let raw: String = cs[val_start..n].iter().collect();
                let v = if q == '\'' {
                    BuildValue::LiteralScalar(raw)
                } else {
                    BuildValue::Scalar(raw)
                };
                store(key, v, path, vars, default_flags);
                break;
            }
            let raw: String = cs[val_start..i].iter().collect();
            i += 1;
            let v = if q == '\'' {
                BuildValue::LiteralScalar(raw)
            } else {
                BuildValue::Scalar(raw)
            };
            store(key, v, path, vars, default_flags);
        } else {
            let val_start = i;
            while i < n && !cs[i].is_whitespace() && cs[i] != '#' {
                i += 1;
            }
            let raw: String = cs[val_start..i].iter().collect();
            store(key, BuildValue::Scalar(raw), path, vars, default_flags);
        }
    }
    ok
}

/// Reads one `package.env` env file (make.conf syntax). Only build
/// vars are kept; `None` if unreadable, a symlink, or structurally broken.
pub(crate) fn load_env_file(path: &Path) -> Option<Vec<(String, BuildValue)>> {
    if !path.is_file() {
        return None;
    }
    let text = match crate::read_to_string_nofollow(path) {
        Ok(t) => t,
        Err(e) if crate::is_symlink_open_error(&e) => return None,
        Err(_) => return None,
    };
    let mut vars: HashMap<String, BuildValue> = HashMap::new();
    let mut defaults: Vec<String> = Vec::new();
    if !parse_into(&text, path, &mut vars, &mut defaults) {
        return None;
    }
    if !defaults.is_empty() {
        warn(path, DEFAULT_FLAG_KEY, "only valid in make.conf (ignored)");
    }
    Some(
        BUILD_VARS
            .iter()
            .filter_map(|k| vars.remove(*k).map(|v| (k.to_string(), v)))
            .collect(),
    )
}

impl Config {
    /// Is `name` on in `FEATURES`? Last mention wins, `-name` is off.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn has_feature(&self, name: &str) -> bool {
        let mut on = false;
        for t in &self.features {
            if t == name {
                on = true;
            } else if t.strip_prefix('-') == Some(name) {
                on = false;
            }
        }
        on
    }

    /// This config with `extra` vars laid over it (same key replaced),
    /// `files` appended to the source list. `BUILD_VARS` order kept.
    pub(crate) fn layered(&self, extra: &[(String, BuildValue)], files: &[PathBuf]) -> Config {
        let mut vars: HashMap<String, BuildValue> = self.build_vars.iter().cloned().collect();
        for (k, v) in extra {
            vars.insert(k.clone(), v.clone());
        }
        let mut out = self.clone();
        out.build_vars = BUILD_VARS
            .iter()
            .filter_map(|k| vars.remove(*k).map(|v| (k.to_string(), v)))
            .collect();
        out.files.extend(files.iter().cloned());
        out
    }
}

fn warn(path: &Path, key: &str, msg: &str) {
    eprintln!(
        "{} {}: {}: {}",
        ">>> Warning:".yellow().bold(),
        path.display(),
        key,
        msg
    );
}

// ── generated makepkg.conf override ───────────────────────────────────────────

/// Escapes for a double-quoted bash string, leaving `$` alone so
/// `-j$(nproc)` still expands.
fn esc_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if c == '"' || c == '`' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Same, but for a value that came from single quotes in make.conf --
/// `$`/`` ` `` must NOT survive into the generated double-quoted
/// string, or a literal `$FOO` would suddenly expand.
fn esc_double_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if c == '\\' || c == '"' || c == '`' || c == '$' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Tokens allowed inside `BUILDENV=()`/`OPTIONS=()`.
fn valid_array_token(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "!_-+.".contains(c))
}

/// Portage `FEATURES` that have a makepkg twin:
/// (feature, makepkg array, token, inverted). Anything else (candy, ...) is
/// not a makepkg matter and is skipped here.
const FEATURE_MAP: &[(&str, &str, &str, bool)] = &[
    ("ccache", "BUILDENV", "ccache", false),
    ("distcc", "BUILDENV", "distcc", false),
    ("test", "BUILDENV", "check", false),
    ("nostrip", "OPTIONS", "strip", true),
    ("splitdebug", "OPTIONS", "debug", false),
];

/// `FEATURES` -> `(array, token)` edits; token may start with `!`. Only
/// features named in FEATURES (on, or `-off`) give an edit. A token the
/// user wrote in an explicit BUILDENV/OPTIONS wins over FEATURES.
pub(crate) fn feature_edits(cfg: &Config) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (feat, arr, tok, inv) in FEATURE_MAP {
        let mut state: Option<bool> = None;
        for t in &cfg.features {
            if t.as_str() == *feat {
                state = Some(true);
            } else if t.strip_prefix('-') == Some(*feat) {
                state = Some(false);
            }
        }
        let Some(on) = state else { continue };
        let explicit = cfg.build_vars.iter().any(|(k, v)| {
            k.as_str() == *arr && v.tokens().iter().any(|x| x.trim_start_matches('!') == *tok)
        });
        if explicit {
            continue;
        }
        let enable = on != *inv;
        out.push((
            *arr,
            if enable {
                tok.to_string()
            } else {
                format!("!{}", tok)
            },
        ));
    }
    out
}

/// makepkg.conf that sources the system one then overrides it with
/// this config's build vars. `None` if none are set (the common case).
pub(crate) fn makepkg_override_conf(cfg: &Config) -> Option<String> {
    let edits = feature_edits(cfg);
    if cfg.build_vars.is_empty() && edits.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str("# Generated by aura-emerge from:\n");
    for f in &cfg.files {
        out.push_str(&format!("#   {}\n", f.display()));
    }
    out.push_str("# Do not edit -- rewritten on every build, removed afterwards.\n\n");
    out.push_str(&format!(
        "source {} 2>/dev/null\n\n",
        crate::MAKEPKG_CONF_SYSTEM
    ));

    let mut exports: Vec<&str> = Vec::new();
    for (key, value) in &cfg.build_vars {
        if ARRAY_VARS.contains(&key.as_str()) {
            let tokens = value.tokens();
            if let Some(bad) = tokens.iter().find(|t| !valid_array_token(t)) {
                eprintln!(
                    "{} make.conf: {} contains an unusable entry '{}' - leaving {} as makepkg.conf has it",
                    ">>> Warning:".yellow().bold(),
                    key,
                    bad,
                    key
                );
                continue;
            }
            out.push_str(&format!("{}=({})\n", key, tokens.join(" ")));
            continue;
        }
        let escaped = match value {
            BuildValue::LiteralScalar(s) => esc_double_literal(s),
            _ => esc_double(&value.display()),
        };
        out.push_str(&format!("{}=\"{}\"\n", key, escaped));
        if EXPORT_VARS.contains(&key.as_str()) {
            exports.push(key.as_str());
        }
    }
    if !exports.is_empty() {
        out.push_str(&format!("export {}\n", exports.join(" ")));
    }
    if !edits.is_empty() {
        // Edit the system arrays in place: drop `tok`/`!tok`, add ours.
        out.push_str("\n# FEATURES -> makepkg arrays\n");
        out.push_str(
            "_ae_set() { local -n _a=\"$1\"; local _t=\"${2#!}\" _i; \
for _i in \"${!_a[@]}\"; do [[ \"${_a[_i]#!}\" == \"$_t\" ]] && unset \"_a[_i]\"; done; \
_a+=(\"$2\"); }\n",
        );
        for (arr, tok) in &edits {
            out.push_str(&format!("_ae_set {} '{}'\n", arr, tok));
        }
    }
    Some(out)
}

// ── EMERGE_DEFAULT_OPTS: argv splicing and conflict detection ─────────────────

/// Flags accepted in `EMERGE_DEFAULT_OPTS`, with whether each takes a
/// value. Allowlist, not denylist: a default silently turning every
/// run into `--unmerge` isn't worth risking; anything missing here
/// still works on the command line.
const ALLOWED_DEFAULTS: &[(&str, bool)] = &[
    ("--ask", false),
    ("--verbose", false),
    ("--quiet", false),
    ("--noreplace", false),
    ("--oneshot", false),
    ("--aur", false),
    ("--abs", false),
    ("--repos", false),
    ("--skippgp", false),
    ("--autopgp", false),
    ("--no-sandbox", false),
    ("--unshare-net-build", false),
    ("--edit", false),
    ("--skip-srcinfo-regen", false),
    ("--pkgbuild-view", false),
    ("--devel", false),
    ("--keep-going", true),
    ("--sudoloop", false),
    ("--err-install", false),
    ("--refresh", false),
    ("--deep", true),
    ("--newuse", false),
    ("--tree", true),
    ("--columns", false),
    ("--nospinner", false),
    ("--debug", true),
    ("--verbose-conflicts", false),
    ("--searchdesc", false),
    ("--skipfirst", false),
    ("--exclude", true),
    ("--color", true),
    ("--jobs", true),
    ("--jobsr", true),
    ("--jobsa", true),
    ("--load-average", true),
    ("--backtrack", true),
    ("--with-bdeps", true),
    ("--quiet-build", true),
];

/// Flag pairs that can't both be in effect, with the reason shown when
/// they are.
const CONFLICTS: &[(&str, &str, &str)] = &[
    ("--aur", "--abs", "they name two different build sources for the same package"),
    ("--aur", "--repos", "one forces the AUR, the other forbids it"),
    (
        "--no-sandbox",
        "--unshare-net-build",
        "--unshare-net-build drops the build's network inside the bwrap sandbox, which --no-sandbox turns off entirely",
    ),
    ("--skippgp", "--autopgp", "one skips PGP checks, the other imports keys to satisfy them"),
];

fn base_of(token: &str) -> &str {
    token.split('=').next().unwrap_or(token)
}

/// Finds a conflicting pair inside one token list.
fn find_conflict(tokens: &[String]) -> Option<(&'static str, &'static str, &'static str)> {
    for (a, b, why) in CONFLICTS {
        let has_a = tokens.iter().any(|t| base_of(t) == *a);
        let has_b = tokens.iter().any(|t| base_of(t) == *b);
        if has_a && has_b {
            return Some((a, b, why));
        }
    }
    None
}

/// Whether `token` conflicts with anything already in `tokens`.
fn conflicts_with(token: &str, tokens: &[String]) -> Option<(&'static str, &'static str)> {
    let base = base_of(token);
    for (a, b, _) in CONFLICTS {
        let other = if base == *a {
            *b
        } else if base == *b {
            *a
        } else {
            continue;
        };
        if tokens.iter().any(|t| base_of(t) == other) {
            return Some((if base == *a { *a } else { *b }, other));
        }
    }
    None
}

/// Builds the argv clap will parse: `EMERGE_DEFAULT_OPTS` first, then the
/// real command line, so an explicitly typed flag always wins.
///
/// Dropped from the config side: everything, if `--ignore-default-opts`
/// was typed; flags not in `ALLOWED_DEFAULTS`; and a flag that
/// conflicts with one typed on the command line (the more specific
/// intent). A conflict within the command line, or left inside the
/// config itself, is a hard error.
pub(crate) fn build_argv(argv: &[String], cfg: &Config) -> Vec<String> {
    let cli: Vec<String> = argv.iter().skip(1).cloned().collect();

    if let Some((a, b, why)) = find_conflict(&cli) {
        eprintln!(
            "{} {} and {} are mutually exclusive - {}.",
            ">>> Error:".red().bold(),
            a,
            b,
            why
        );
        std::process::exit(1);
    }

    let ignore_defaults = cli.iter().any(|t| base_of(t) == "--ignore-default-opts");
    if ignore_defaults || cfg.default_flags.is_empty() {
        return argv.to_vec();
    }

    let source = cfg
        .files
        .last()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| SYSTEM_CONF.to_string());

    if let Some((a, b, why)) = find_conflict(&cfg.default_flags) {
        eprintln!(
            "{} {}: EMERGE_DEFAULT_OPTS sets both {} and {} - {}.",
            ">>> Error:".red().bold(),
            source,
            a,
            b,
            why
        );
        std::process::exit(1);
    }

    let mut kept: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < cfg.default_flags.len() {
        let token = cfg.default_flags[i].clone();
        let base = base_of(&token).to_string();

        let Some((_, takes_value)) = ALLOWED_DEFAULTS.iter().find(|(n, _)| *n == base) else {
            eprintln!(
                "{} {}: '{}' is not accepted in EMERGE_DEFAULT_OPTS - ignoring it (pass it on the command line instead).",
                ">>> Warning:".yellow().bold(),
                source,
                token
            );
            i += 1;
            // Skip a value that was clearly meant for it.
            if cfg
                .default_flags
                .get(i)
                .map(|t| !t.starts_with('-'))
                .unwrap_or(false)
            {
                i += 1;
            }
            continue;
        };

        let inline_value = token.contains('=');
        let value = if *takes_value && !inline_value {
            let v = cfg.default_flags.get(i + 1).cloned();
            match v {
                Some(v) if !v.starts_with('-') => Some(v),
                _ => {
                    eprintln!(
                        "{} {}: '{}' in EMERGE_DEFAULT_OPTS needs a value - ignoring it.",
                        ">>> Warning:".yellow().bold(),
                        source,
                        token
                    );
                    i += 1;
                    continue;
                }
            }
        } else {
            None
        };

        if let Some((from_cfg, from_cli)) = conflicts_with(&token, &cli) {
            println!(
                "{} {} is set on the command line, so {} from {} is ignored for this run.",
                ">>>".yellow().bold(),
                from_cli,
                from_cfg,
                source
            );
            i += 1 + value.is_some() as usize;
            continue;
        }

        kept.push(token);
        if let Some(v) = value {
            kept.push(v);
        }
        i += 1 + if *takes_value && !inline_value { 1 } else { 0 };
    }

    let mut out: Vec<String> = vec![argv.first().cloned().unwrap_or_default()];
    out.extend(kept);
    out.extend(cli);
    out
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn parse(text: &str) -> Config {
        let mut vars = HashMap::new();
        let mut flags = Vec::new();
        assert!(parse_into(
            text,
            Path::new("test.conf"),
            &mut vars,
            &mut flags
        ));
        let build_vars = BUILD_VARS
            .iter()
            .filter_map(|k| vars.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect();
        let features = features_of(&vars);
        Config {
            default_flags: flags,
            features,
            build_vars,
            files: vec![PathBuf::from("test.conf")],
        }
    }

    #[test]
    fn default_flags_accept_array_and_string() {
        let a = parse(r#"EMERGE_DEFAULT_OPTS=(--ask --devel)"#);
        let b = parse(r#"EMERGE_DEFAULT_OPTS="--ask --devel""#);
        assert_eq!(a.default_flags, vec!["--ask", "--devel"]);
        assert_eq!(a.default_flags, b.default_flags);
    }

    #[test]
    fn features_map_to_makepkg_arrays() {
        let c = parse("FEATURES=\"ccache -test nostrip candy\"\n");
        assert_eq!(
            feature_edits(&c),
            vec![
                ("BUILDENV", "ccache".to_string()),
                ("BUILDENV", "!check".to_string()),
                ("OPTIONS", "!strip".to_string()),
            ]
        );
        let conf = makepkg_override_conf(&c).unwrap();
        assert!(conf.contains("_ae_set BUILDENV 'ccache'"));
        assert!(conf.contains("_ae_set OPTIONS '!strip'"));
    }

    #[test]
    fn explicit_buildenv_beats_features() {
        let c = parse("FEATURES=ccache\nBUILDENV=(!ccache color)\n");
        assert!(feature_edits(&c).is_empty());
    }

    #[test]
    fn features_are_read_but_not_build_vars() {
        let cfg = parse("FEATURES=\"candy ccache\"\nCFLAGS=\"-O2\"\n");
        assert_eq!(cfg.features, vec!["candy", "ccache"]);
        assert!(cfg.has_feature("candy"));
        assert!(!cfg.has_feature("sandbox"));
        assert_eq!(cfg.build_vars.len(), 1);
        assert_eq!(cfg.build_vars[0].0, "CFLAGS");
    }

    #[test]
    fn feature_minus_turns_off_last_wins() {
        assert!(!parse("FEATURES=(candy -candy)").has_feature("candy"));
        assert!(parse("FEATURES=(-candy candy)").has_feature("candy"));
        assert!(!parse("CFLAGS=\"-O2\"").has_feature("candy"));
    }

    #[test]
    fn build_keys_read_flat() {
        let cfg = parse("CFLAGS=\"-O2\"\n");
        assert_eq!(cfg.build_vars.len(), 1);
        assert_eq!(cfg.build_vars[0].1.display(), "-O2");
    }

    #[test]
    fn scalars_and_lists_are_interchangeable() {
        let as_list = parse("OPTIONS=(strip !debug)");
        let as_string = parse(r#"OPTIONS="strip !debug""#);
        assert_eq!(as_list.build_vars[0].1.display(), "strip !debug");
        assert_eq!(
            as_list.build_vars[0].1.tokens(),
            as_string.build_vars[0].1.tokens()
        );
    }

    #[test]
    fn bare_unquoted_scalar_works() {
        let cfg = parse("MAKEFLAGS=-j16\n");
        assert_eq!(cfg.build_vars[0].1.display(), "-j16");
    }

    #[test]
    fn single_quoted_value_stays_literal_when_reembedded() {
        let cfg = parse("CFLAGS='$FOO'\n");
        let conf = makepkg_override_conf(&cfg).unwrap();
        assert!(conf.contains(r#"CFLAGS="\$FOO""#));
    }

    #[test]
    fn unknown_keys_are_ignored_not_fatal() {
        let cfg = parse("NONSENSE=\"x\"\nALSO_NONSENSE=\"y\"\nCFLAGS=\"-O2\"\n");
        assert_eq!(cfg.build_vars.len(), 1);
        assert_eq!(cfg.build_vars[0].0, "CFLAGS");
    }

    #[test]
    fn unterminated_quote_is_reported_not_panicked() {
        let mut vars = HashMap::new();
        let mut flags = Vec::new();
        assert!(!parse_into(
            "CFLAGS=\"unclosed\n",
            Path::new("t.conf"),
            &mut vars,
            &mut flags
        ));
    }

    #[test]
    fn unterminated_array_is_reported_not_panicked() {
        let mut vars = HashMap::new();
        let mut flags = Vec::new();
        assert!(!parse_into(
            "OPTIONS=(strip !debug\n",
            Path::new("t.conf"),
            &mut vars,
            &mut flags
        ));
    }

    #[test]
    fn generated_conf_sources_system_then_overrides() {
        let cfg = parse("CFLAGS=\"-O2\"\nMAKEFLAGS=\"-j$(nproc)\"\nOPTIONS=(strip !debug)\nNINJAFLAGS=\"-j4\"\n");
        let conf = makepkg_override_conf(&cfg).expect("build vars set");
        let src = conf.find("source ").expect("sources the system conf");
        // Arrays stay arrays, scalars stay quoted, $() survives.
        assert!(conf.contains("OPTIONS=(strip !debug)"));
        assert!(conf.contains(r#"MAKEFLAGS="-j$(nproc)""#));
        assert!(conf.contains("export NINJAFLAGS"));
        // Overrides must come after the source line to actually win.
        assert!(conf.find("CFLAGS=").unwrap() > src);
    }

    #[test]
    fn no_build_vars_means_no_generated_conf() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--ask)"#);
        assert!(makepkg_override_conf(&cfg).is_none());
    }

    #[test]
    fn cli_flag_wins_over_conflicting_config_default() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--aur)"#);
        let argv = vec![
            "emerge".to_string(),
            "--abs".to_string(),
            "nano".to_string(),
        ];
        let out = build_argv(&argv, &cfg);
        assert!(!out.iter().any(|t| t == "--aur"));
        assert!(out.iter().any(|t| t == "--abs"));
    }

    #[test]
    fn config_defaults_precede_the_command_line() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--pkgbuild-view)"#);
        let argv = vec!["emerge".to_string(), "nano".to_string()];
        assert_eq!(
            build_argv(&argv, &cfg),
            vec!["emerge", "--pkgbuild-view", "nano"]
        );
    }

    #[test]
    fn ignore_default_opts_drops_them_all() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--pkgbuild-view)"#);
        let argv = vec!["emerge".to_string(), "--ignore-default-opts".to_string()];
        assert_eq!(build_argv(&argv, &cfg), argv);
    }

    #[test]
    fn action_flags_are_not_accepted_as_defaults() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--unmerge --ask)"#);
        let out = build_argv(&vec!["emerge".to_string()], &cfg);
        assert!(!out.iter().any(|t| t == "--unmerge"));
        assert!(out.iter().any(|t| t == "--ask"));
    }

    #[test]
    fn valued_default_flag_keeps_its_value() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS=(--exclude linux)"#);
        let out = build_argv(&vec!["emerge".to_string(), "-u".to_string()], &cfg);
        assert_eq!(out, vec!["emerge", "--exclude", "linux", "-u"]);
    }

    #[test]
    fn abs_and_repos_are_not_a_conflict() {
        assert!(find_conflict(&["--abs".to_string(), "--repos".to_string()]).is_none());
    }
}
