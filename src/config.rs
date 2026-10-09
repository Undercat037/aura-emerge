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

use std::collections::{HashMap, HashSet};
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

/// Message colour overrides, read by `theme`; never reaches makepkg.
const COLORS_KEY: &str = "COLORS";

/// Roles `COLORS` can name.
const COLOR_ROLES: &[&str] = &["ok", "warn", "error", "info", "special"];

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
    /// `COLORS` roles (`ok`, `warn`, `error`, `info`, `special`) with their colour.
    pub(crate) colors: Vec<(String, colored::Color)>,
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
    let colors = colors_of(&vars, Path::new(SYSTEM_CONF));

    Config {
        default_flags,
        features,
        colors,
        build_vars,
        files,
    }
}

fn features_of(vars: &HashMap<String, BuildValue>) -> Vec<String> {
    vars.get(FEATURES_KEY)
        .map(BuildValue::tokens)
        .unwrap_or_default()
}

/// `#RRGGBB`, `#RGB`, or one of the 16 ANSI names (`red`, `bright-cyan`,
/// `gray`, ...). Case and `_` vs `-` don't matter.
pub(crate) fn parse_color(s: &str) -> Option<colored::Color> {
    use colored::Color::*;
    let t = s.trim().to_ascii_lowercase().replace('_', "-");
    if let Some(h) = t.strip_prefix('#') {
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let full: String = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect(),
            6 => h.to_string(),
            _ => return None,
        };
        let v = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
        return Some(TrueColor {
            r: v(0)?,
            g: v(2)?,
            b: v(4)?,
        });
    }
    Some(match t.as_str() {
        "black" => Black,
        "red" => Red,
        "green" => Green,
        "yellow" => Yellow,
        "blue" => Blue,
        "magenta" => Magenta,
        "cyan" => Cyan,
        "white" => White,
        "gray" | "grey" | "bright-black" => BrightBlack,
        "bright-red" => BrightRed,
        "bright-green" => BrightGreen,
        "bright-yellow" => BrightYellow,
        "bright-blue" => BrightBlue,
        "bright-magenta" => BrightMagenta,
        "bright-cyan" => BrightCyan,
        "bright-white" => BrightWhite,
        _ => return None,
    })
}

/// `COLORS="role=colour ..."` -> (role, colour). Bad entries warn and
/// are skipped; the last mention of a role wins.
fn colors_of(vars: &HashMap<String, BuildValue>, path: &Path) -> Vec<(String, colored::Color)> {
    let mut out: Vec<(String, colored::Color)> = Vec::new();
    let Some(v) = vars.get(COLORS_KEY) else {
        return out;
    };
    for tok in v.tokens() {
        let Some((role, color)) = tok.split_once('=') else {
            warn(
                path,
                COLORS_KEY,
                &format!("'{}' should look like role=colour (ignored)", tok),
            );
            continue;
        };
        if !COLOR_ROLES.contains(&role) {
            warn(
                path,
                COLORS_KEY,
                &format!(
                    "unknown role '{}' (use {}) - ignored",
                    role,
                    COLOR_ROLES.join(", ")
                ),
            );
            continue;
        }
        let color = color.trim_matches(|c| c == '"' || c == '\'');
        let Some(c) = parse_color(color) else {
            warn(
                path,
                COLORS_KEY,
                &format!(
                    "'{}' is not a colour (use #RRGGBB or a name like red, bright-cyan) - ignored",
                    color
                ),
            );
            continue;
        };
        out.retain(|(r, _)| r != role);
        out.push((role.to_string(), c));
    }
    out
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
    } else if key == FEATURES_KEY || key == COLORS_KEY || BUILD_VARS.contains(&key.as_str()) {
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

/// Value kinds a default flag can carry. Each one is checked (and
/// normalised) here, so a typo in make.conf is a warning, not a clap
/// error on every run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Val {
    /// Whole number (`--jobs 4`).
    Count,
    /// Positive decimal (`--load-average 4.5`).
    Load,
    /// `y` / `n` (`--quiet-build n`). Stored as `y` or `n`.
    YesNo,
    /// `y` / `n` / `auto`.
    Color,
    /// Package names, comma-separated (`--exclude linux,nvidia`).
    Atoms,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arity {
    /// Plain switch. `--ask=y` is on, `--ask=n` is off.
    Switch,
    /// Value required: `--jobs 4` or `--jobs=4`.
    Value(Val),
    /// Bare, `--deep=3`, or `--deep 3` (config only).
    Optional(Val),
}

struct FlagSpec {
    long: &'static str,
    short: Option<char>,
    /// Spellings of one option share a key (`--jobs` = `--jobsr`).
    key: &'static str,
    arity: Arity,
    /// Values add up instead of replacing (`--exclude`).
    repeat: bool,
    /// Applied only if one of these keys was typed on the command line.
    only_with: &'static [&'static str],
}

const fn sw(long: &'static str, short: Option<char>) -> FlagSpec {
    FlagSpec {
        long,
        short,
        key: long,
        arity: Arity::Switch,
        repeat: false,
        only_with: &[],
    }
}

const fn scoped(
    long: &'static str,
    short: Option<char>,
    only_with: &'static [&'static str],
) -> FlagSpec {
    FlagSpec {
        long,
        short,
        key: long,
        arity: Arity::Switch,
        repeat: false,
        only_with,
    }
}

const fn valued(long: &'static str, key: &'static str, kind: Val) -> FlagSpec {
    FlagSpec {
        long,
        short: None,
        key,
        arity: Arity::Value(kind),
        repeat: false,
        only_with: &[],
    }
}

/// Flags accepted in `EMERGE_DEFAULT_OPTS`. Only flags that change what
/// a run does are listed; Portage options aura-emerge accepts as no-ops
/// (`--newuse`, `--with-bdeps`, `--backtrack`, ...) are left out on
/// purpose, so a default can't look active while doing nothing. They
/// still parse on the command line. Allowlist, not denylist: a
/// default silently turning every run into `--unmerge` isn't worth
/// risking, so actions (`-u`, `-C`, `-c`, `-p`, `-e`, ...) stay out;
/// anything missing here still works on the command line.
///
/// Flags that only make sense next to an action (`--searchdesc` with
/// `-s`, `--skipfirst` with `--resume`) carry `only_with`: they apply
/// when that action is typed and are ignored otherwise.
static SPECS: &[FlagSpec] = &[
    // output / interaction
    sw("--ask", Some('a')),
    sw("--verbose", Some('v')),
    sw("--nospinner", None),
    sw("--debug", None),
    sw("--tree", Some('t')),
    FlagSpec {
        long: "--deep",
        short: Some('D'),
        key: "--deep",
        arity: Arity::Optional(Val::Count),
        repeat: false,
        only_with: &[],
    },
    valued("--color", "--color", Val::Color),
    // what gets installed, and how
    sw("--noreplace", Some('n')),
    sw("--oneshot", Some('1')),
    sw("--with-optdeps", None),
    sw("--err-install", None),
    sw("--refresh", None),
    sw("--devel", None),
    sw("--keep-going", None),
    sw("--sudoloop", None),
    FlagSpec {
        long: "--exclude",
        short: None,
        key: "--exclude",
        arity: Arity::Value(Val::Atoms),
        repeat: true,
        only_with: &[],
    },
    // source selection
    sw("--aur", None),
    sw("--abs", None),
    sw("--repos", None),
    // build / security
    sw("--skippgp", None),
    sw("--autopgp", None),
    sw("--no-sandbox", None),
    sw("--unshare-net-build", None),
    sw("--edit", None),
    sw("--skip-srcinfo-regen", None),
    sw("--pkgbuild-view", None),
    valued("--quiet-build", "--quiet-build", Val::YesNo),
    // parallelism
    valued("--jobs", "--jobsr", Val::Count),
    valued("--jobsr", "--jobsr", Val::Count),
    valued("--jobsa", "--jobsa", Val::Count),
    valued("--load-average", "--load-average", Val::Load),
    // action modifiers
    scoped("--searchdesc", Some('S'), &["--search"]),
    scoped("--skipfirst", None, &["--resume"]),
];

/// Actions the command line can type that a scoped default hangs on.
/// (long, short) - only used to notice them, never accepted as defaults.
const SCOPE_ONLY: &[(&str, Option<char>)] = &[("--search", Some('s')), ("--resume", None)];

fn spec_long(name: &str) -> Option<&'static FlagSpec> {
    SPECS.iter().find(|s| s.long == name)
}

fn spec_short(c: char) -> Option<&'static FlagSpec> {
    SPECS.iter().find(|s| s.short == Some(c))
}

/// `y`/`n` in every spelling Portage users type.
pub(crate) fn parse_yes_no(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "true" | "1" | "on" | "always" => Some(true),
        "n" | "no" | "false" | "0" | "off" | "never" => Some(false),
        _ => None,
    }
}

/// Checks one value; returns it in the form clap will get.
fn check_value(kind: Val, raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    match kind {
        Val::Count => raw
            .parse::<u32>()
            .map(|n| n.to_string())
            .map_err(|_| "expected a whole number"),
        Val::Load => match raw.parse::<f32>() {
            Ok(f) if f.is_finite() && f > 0.0 => Ok(raw.to_string()),
            _ => Err("expected a number greater than 0"),
        },
        Val::YesNo => match parse_yes_no(raw) {
            Some(true) => Ok("y".to_string()),
            Some(false) => Ok("n".to_string()),
            None => Err("expected y or n"),
        },
        Val::Color => {
            if raw.eq_ignore_ascii_case("auto") {
                return Ok("auto".to_string());
            }
            match parse_yes_no(raw) {
                Some(true) => Ok("y".to_string()),
                Some(false) => Ok("n".to_string()),
                None => Err("expected y, n or auto"),
            }
        }
        Val::Atoms => {
            if !raw.is_empty()
                && raw
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "@._+-/,".contains(c))
            {
                Ok(raw.to_string())
            } else {
                Err("expected package names")
            }
        }
    }
}

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

/// One accepted default, already in the form clap will see.
struct Entry {
    spec: &'static FlagSpec,
    token: String,
}

fn push_entry(out: &mut Vec<Entry>, spec: &'static FlagSpec, value: Option<String>) {
    if !spec.repeat {
        out.retain(|e| e.spec.key != spec.key);
    }
    let token = match value {
        Some(v) => format!("{}={}", spec.long, v),
        None => spec.long.to_string(),
    };
    out.push(Entry { spec, token });
}

fn warn_default(source: &str, msg: &str) {
    eprintln!("{} {}: {}", ">>> Warning:".yellow().bold(), source, msg);
}

/// `EMERGE_DEFAULT_OPTS` tokens -> accepted entries. Within the list the
/// last mention of a flag wins; `--flag=n` switches an earlier one off.
/// Bad flags and bad values warn and are skipped; nothing here is fatal.
fn parse_defaults(raw: &[String], source: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut i = 0usize;
    while i < raw.len() {
        let tok = raw[i].as_str();
        i += 1;

        if tok.len() > 2 && tok.starts_with("--") {
            let (name, inline) = match tok.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (tok, None),
            };
            let Some(spec) = spec_long(name) else {
                warn_default(
                    source,
                    &format!(
                        "'{}' is not accepted in EMERGE_DEFAULT_OPTS - ignoring it (pass it on the command line instead).",
                        tok
                    ),
                );
                // Skip a value that was clearly meant for it.
                if inline.is_none() && raw.get(i).map(|t| !t.starts_with('-')).unwrap_or(false) {
                    i += 1;
                }
                continue;
            };

            let value: Option<String> = match spec.arity {
                Arity::Switch => match inline.map(parse_yes_no) {
                    None | Some(Some(true)) => None,
                    Some(Some(false)) => {
                        out.retain(|e| e.spec.key != spec.key);
                        continue;
                    }
                    Some(None) => {
                        warn_default(
                            source,
                            &format!("'{}': expected y or n - ignoring it.", tok),
                        );
                        continue;
                    }
                },
                Arity::Value(kind) => {
                    let raw_v = match inline {
                        Some(v) => v.to_string(),
                        None => match raw.get(i) {
                            Some(t) if !t.starts_with('-') => {
                                i += 1;
                                t.clone()
                            }
                            _ => {
                                warn_default(
                                    source,
                                    &format!(
                                        "'{}' in EMERGE_DEFAULT_OPTS needs a value - ignoring it.",
                                        tok
                                    ),
                                );
                                continue;
                            }
                        },
                    };
                    match check_value(kind, &raw_v) {
                        Ok(v) => Some(v),
                        Err(why) => {
                            warn_default(
                                source,
                                &format!(
                                    "'{}' has an invalid value '{}' ({}) - ignoring it.",
                                    spec.long, raw_v, why
                                ),
                            );
                            continue;
                        }
                    }
                }
                Arity::Optional(kind) => {
                    let raw_v = match inline {
                        Some(v) => Some(v.to_string()),
                        None => match raw.get(i) {
                            Some(t) if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) => {
                                i += 1;
                                Some(t.clone())
                            }
                            _ => None,
                        },
                    };
                    match raw_v {
                        None => None,
                        Some(r) => match check_value(kind, &r) {
                            Ok(v) => Some(v),
                            Err(why) => {
                                warn_default(
                                    source,
                                    &format!(
                                        "'{}' has an invalid value '{}' ({}) - ignoring it.",
                                        spec.long, r, why
                                    ),
                                );
                                continue;
                            }
                        },
                    }
                }
            };
            push_entry(&mut out, spec, value);
        } else if tok.len() > 1 && tok.starts_with('-') && tok != "--" {
            // Short cluster: `-avt` = --ask --verbose --tree.
            for c in tok[1..].chars() {
                match spec_short(c) {
                    Some(spec) if !matches!(spec.arity, Arity::Value(_)) => {
                        push_entry(&mut out, spec, None)
                    }
                    _ => warn_default(
                        source,
                        &format!(
                            "'-{}' is not accepted in EMERGE_DEFAULT_OPTS - ignoring it (use the long form).",
                            c
                        ),
                    ),
                }
            }
        } else {
            warn_default(
                source,
                &format!(
                    "'{}' is not accepted in EMERGE_DEFAULT_OPTS - ignoring it.",
                    tok
                ),
            );
        }
    }
    out
}

/// Reads the typed command line once: returns it normalised and the
/// set of option keys it names (longs, short clusters, aliases alike).
///
/// Normalising: `--ask=y` becomes `--ask`; `--ask=n` is dropped and
/// counts as typed, which is how one run switches a default off (no
/// `--ignore-default-opts` needed). Everything after `--` is left alone.
fn scan_cli(raw: &[String]) -> (Vec<String>, HashSet<&'static str>) {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut typed: HashSet<&'static str> = HashSet::new();
    let mut rest = false;

    for tok in raw {
        if rest {
            out.push(tok.clone());
            continue;
        }
        if tok == "--" {
            rest = true;
            out.push(tok.clone());
            continue;
        }
        if tok.starts_with("--") {
            let (name, inline) = match tok.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (tok.as_str(), None),
            };
            if let Some(spec) = spec_long(name) {
                typed.insert(spec.key);
                if spec.arity == Arity::Switch {
                    match inline.map(parse_yes_no) {
                        Some(Some(true)) => {
                            out.push(spec.long.to_string());
                            continue;
                        }
                        Some(Some(false)) => continue,
                        _ => {}
                    }
                }
            } else if let Some((k, _)) = SCOPE_ONLY.iter().find(|(l, _)| *l == name) {
                typed.insert(*k);
            }
            out.push(tok.clone());
        } else if tok.len() > 1 && tok.starts_with('-') {
            let cluster = tok[1..].split('=').next().unwrap_or("");
            for c in cluster.chars() {
                if let Some(spec) = spec_short(c) {
                    typed.insert(spec.key);
                } else if let Some((k, _)) = SCOPE_ONLY.iter().find(|(_, s)| *s == Some(c)) {
                    typed.insert(*k);
                }
            }
            out.push(tok.clone());
        } else {
            out.push(tok.clone());
        }
    }
    (out, typed)
}

/// Builds the argv clap will parse: `EMERGE_DEFAULT_OPTS` first, then the
/// real command line, so an explicitly typed flag always wins.
///
/// clap rejects a flag given twice, so a default is dropped whenever the
/// same option was typed - in any spelling (`-a`/`--ask`, `--jobs`/
/// `--jobsr`) - and a default valued flag never meets a typed value.
///
/// Dropped from the config side: everything, if `--ignore-default-opts`
/// was typed; flags not in `SPECS`; invalid values; scoped flags whose
/// action wasn't typed; and a flag that conflicts with one typed on the
/// command line (the more specific intent). A conflict within the
/// command line, or left inside the config itself, is a hard error.
pub(crate) fn build_argv(argv: &[String], cfg: &Config) -> Vec<String> {
    let argv0 = argv.first().cloned().unwrap_or_default();
    let raw_cli: Vec<String> = argv.iter().skip(1).cloned().collect();
    let (cli, typed) = scan_cli(&raw_cli);

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

    let mut out: Vec<String> = vec![argv0];

    let ignore_defaults = cli.iter().any(|t| base_of(t) == "--ignore-default-opts");
    if ignore_defaults || cfg.default_flags.is_empty() {
        out.extend(cli);
        return out;
    }

    let source = cfg
        .files
        .last()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| SYSTEM_CONF.to_string());

    let entries = parse_defaults(&cfg.default_flags, &source);

    let own: Vec<String> = entries.iter().map(|e| e.token.clone()).collect();
    if let Some((a, b, why)) = find_conflict(&own) {
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

    for e in entries {
        let spec = e.spec;
        if !spec.only_with.is_empty() && !spec.only_with.iter().any(|k| typed.contains(k)) {
            continue;
        }
        if !spec.repeat && typed.contains(&spec.key) {
            continue;
        }
        if let Some((from_cfg, from_cli)) = conflicts_with(&e.token, &cli) {
            println!(
                "{} {} is set on the command line, so {} from {} is ignored for this run.",
                ">>>".yellow().bold(),
                from_cli,
                from_cfg,
                source
            );
            continue;
        }
        out.push(e.token);
    }

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
        let colors = colors_of(&vars, Path::new("test.conf"));
        Config {
            default_flags: flags,
            features,
            colors,
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
        assert_eq!(out, vec!["emerge", "--exclude=linux", "-u"]);
    }

    #[test]
    fn abs_and_repos_are_not_a_conflict() {
        assert!(find_conflict(&["--abs".to_string(), "--repos".to_string()]).is_none());
    }

    fn run(defaults: &str, cli: &[&str]) -> Vec<String> {
        let cfg = parse(&format!("EMERGE_DEFAULT_OPTS=\"{}\"\n", defaults));
        let mut argv = vec!["emerge".to_string()];
        argv.extend(cli.iter().map(|s| s.to_string()));
        build_argv(&argv, &cfg)
    }

    #[test]
    fn tree_and_deep_need_no_value() {
        assert_eq!(
            run("--tree --deep", &["nano"]),
            vec!["emerge", "--tree", "--deep", "nano"]
        );
    }

    #[test]
    fn deep_takes_optional_number() {
        assert_eq!(run("--deep=3", &[]), vec!["emerge", "--deep=3"]);
        assert_eq!(
            run("--deep 3 --ask", &[]),
            vec!["emerge", "--deep=3", "--ask"]
        );
    }

    #[test]
    fn typed_short_flag_replaces_long_default() {
        // Without this clap fails with "cannot be used multiple times".
        assert_eq!(
            run("--ask --verbose", &["-a", "nano"]),
            vec!["emerge", "--verbose", "-a", "nano"]
        );
        assert_eq!(run("--ask", &["-uav"]), vec!["emerge", "-uav"]);
    }

    #[test]
    fn typed_value_replaces_default_value() {
        assert_eq!(
            run("--jobs 4 --jobsa 3", &["--jobsr", "2"]),
            vec!["emerge", "--jobsa=3", "--jobsr", "2"]
        );
        assert_eq!(
            run("--color=y", &["--color=n"]),
            vec!["emerge", "--color=n"]
        );
    }

    #[test]
    fn exclude_adds_up() {
        assert_eq!(
            run("--exclude linux", &["--exclude", "nvidia"]),
            vec!["emerge", "--exclude=linux", "--exclude", "nvidia"]
        );
    }

    #[test]
    fn y_n_values_are_normalised() {
        assert_eq!(
            run("--quiet-build=no --color always", &[]),
            vec!["emerge", "--quiet-build=n", "--color=y"]
        );
    }

    #[test]
    fn bad_values_are_dropped_not_fatal() {
        assert_eq!(
            run("--jobs abc --jobsa x --ask", &[]),
            vec!["emerge", "--ask"]
        );
        assert_eq!(run("--jobs", &[]), vec!["emerge"]);
    }

    #[test]
    fn switch_equals_n_turns_it_off() {
        assert_eq!(
            run("--ask --ask=n --verbose", &[]),
            vec!["emerge", "--verbose"]
        );
    }

    #[test]
    fn typed_equals_n_switches_a_default_off_for_one_run() {
        assert_eq!(
            run("--ask --verbose", &["--ask=n", "nano"]),
            vec!["emerge", "--verbose", "nano"]
        );
        assert_eq!(
            run("", &["--ask=y", "nano"]),
            vec!["emerge", "--ask", "nano"]
        );
    }

    #[test]
    fn last_default_wins_inside_the_config() {
        assert_eq!(run("--jobs 2 --jobsr 8", &[]), vec!["emerge", "--jobsr=8"]);
    }

    #[test]
    fn short_cluster_in_config() {
        assert_eq!(
            run("-avt", &[]),
            vec!["emerge", "--ask", "--verbose", "--tree"]
        );
        assert_eq!(run("-s", &[]), vec!["emerge"]);
    }

    #[test]
    fn scoped_defaults_need_their_action() {
        assert_eq!(run("--searchdesc", &["nano"]), vec!["emerge", "nano"]);
        assert_eq!(
            run("--searchdesc", &["-s", "nano"]),
            vec!["emerge", "--searchdesc", "-s", "nano"]
        );
        assert_eq!(
            run("--skipfirst", &["--resume"]),
            vec!["emerge", "--skipfirst", "--resume"]
        );
        assert_eq!(run("--skipfirst", &["-a"]), vec!["emerge", "-a"]);
    }

    #[test]
    fn flags_that_do_nothing_are_not_accepted() {
        let out = run(
            "--newuse --with-bdeps=y --backtrack 30 --changed-use --ask",
            &[],
        );
        assert_eq!(out, vec!["emerge", "--ask"]);
    }

    #[test]
    fn load_average_takes_a_positive_number() {
        assert_eq!(
            run("--load-average 4.5", &[]),
            vec!["emerge", "--load-average=4.5"]
        );
        assert_eq!(run("--load-average 0 --ask", &[]), vec!["emerge", "--ask"]);
        assert_eq!(run("--load-average=abc", &[]), vec!["emerge"]);
        assert_eq!(
            run("--load-average 8", &["--load-average", "2"]),
            vec!["emerge", "--load-average", "2"]
        );
    }

    #[test]
    fn colors_take_hex_and_ansi_names() {
        let c = parse(
            "COLORS=\"ok=#a6e3a1 warn=Bright-Yellow error=#f00 info=cyan special=#CBA6F7\"\n",
        );
        assert_eq!(c.colors.len(), 5);
        assert_eq!(
            c.colors[0].1,
            colored::Color::TrueColor {
                r: 0xa6,
                g: 0xe3,
                b: 0xa1
            }
        );
        assert_eq!(c.colors[1].1, colored::Color::BrightYellow);
        assert_eq!(
            c.colors[2].1,
            colored::Color::TrueColor { r: 255, g: 0, b: 0 }
        );
        assert_eq!(c.colors[3].1, colored::Color::Cyan);
    }

    #[test]
    fn bad_colors_and_roles_are_skipped() {
        let c = parse("COLORS=\"ok=orange nope=red warn error=#12345 info=blue info=#000\"\n");
        assert_eq!(
            c.colors,
            vec![(
                "info".to_string(),
                colored::Color::TrueColor { r: 0, g: 0, b: 0 }
            )]
        );
    }

    #[test]
    fn colors_array_form_needs_quoted_hex() {
        // A bare `#` starts a comment, so hex goes in quotes.
        let c = parse("COLORS=(ok=\"#a6e3a1\" warn=yellow)\n");
        assert_eq!(c.colors.len(), 2);
    }

    #[test]
    fn colors_are_not_build_vars() {
        let c = parse("COLORS=\"ok=red\"\nCFLAGS=\"-O2\"\n");
        assert_eq!(c.build_vars.len(), 1);
        assert!(makepkg_override_conf(&c).unwrap().contains("CFLAGS"));
    }

    #[test]
    fn args_after_double_dash_are_untouched() {
        assert_eq!(
            run("--ask", &["--", "-a"]),
            vec!["emerge", "--ask", "--", "-a"]
        );
    }
}
