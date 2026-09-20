//! `/etc/emerge/emerge.toml` and `~/.config/emerge/emerge.toml`.
//!
//! Two jobs in one file, like Portage's `make.conf`:
//!   * `EMERGE_DEFAULT_OPTS` -- flags spliced into argv before clap
//!     sees it. `--ignore-default-opts` skips them for one run.
//!   * `[build]` -- `CFLAGS`/`CXXFLAGS`/`LDFLAGS`/`RUSTFLAGS`/
//!     `MAKEFLAGS`/`NINJAFLAGS`/`BUILDENV`/`OPTIONS`, applied via a
//!     generated makepkg.conf passed to makepkg as `--config`.
//!
//! ```toml
//! EMERGE_DEFAULT_OPTS = ["--pkgbuild-view", "--unshare-net-build"]
//!
//! [build]
//! CFLAGS = "-march=native -O2 -pipe"
//! MAKEFLAGS = "-j$(nproc)"
//! OPTIONS = ["strip", "!debug"]
//! ```
//!
//! Scalars and lists are interchangeable: a list is joined with
//! spaces, a string is split on them. Values are shell-expanded when
//! makepkg sources the generated file, so `-j$(nproc)` works; a TOML
//! literal string (`'\$FOO'`) keeps a dollar literal.
//!
//! System file first, then the user one; last to set a key wins.
//! Trust note: a value here becomes shell code run as the build user,
//! the same trust level `/etc/makepkg.conf` already has.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use colored::Colorize;

pub(crate) const SYSTEM_CONF: &str = "/etc/emerge/emerge.toml";

/// Build keys understood in `[build]`.
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

/// A build value: TOML string or array of strings, kept as written so
/// arrays stay arrays in the generated makepkg.conf.
#[derive(Debug, Clone)]
pub(crate) enum BuildValue {
    Scalar(String),
    List(Vec<String>),
}

impl BuildValue {
    /// One-line form, for `--info` and messages.
    pub(crate) fn display(&self) -> String {
        match self {
            BuildValue::Scalar(s) => s.clone(),
            BuildValue::List(v) => v.join(" "),
        }
    }

    /// Tokens, for keys makepkg holds as a bash array.
    fn tokens(&self) -> Vec<String> {
        match self {
            BuildValue::Scalar(s) => s.split_whitespace().map(str::to_string).collect(),
            BuildValue::List(v) => v.clone(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Config {
    /// `EMERGE_DEFAULT_OPTS`, already one token per entry.
    pub(crate) default_flags: Vec<String>,
    /// Build vars actually set, in `BUILD_VARS` order.
    pub(crate) build_vars: Vec<(String, BuildValue)>,
    /// Config files that were read, in precedence order.
    pub(crate) files: Vec<PathBuf>,
}

/// `$XDG_CONFIG_HOME/emerge/emerge.toml`, else `~/.config/emerge/emerge.toml`.
pub(crate) fn user_conf_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("emerge/emerge.toml"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config/emerge/emerge.toml"))
}

/// Reads the system config, then the user one; later wins per key.
/// Missing files are fine; a symlinked one is refused, as elsewhere.
pub(crate) fn load() -> Config {
    let mut vars: HashMap<String, BuildValue> = HashMap::new();
    let mut default_flags: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    let mut paths: Vec<PathBuf> = vec![PathBuf::from(SYSTEM_CONF)];
    if let Some(user) = user_conf_path() {
        paths.push(user);
    }

    for path in paths {
        if !path.is_file() {
            continue;
        }
        if !crate::is_safe_path(&path.to_string_lossy()) {
            eprintln!(
                "{} {} is a symlink - refusing to read",
                ">>> Warning:".yellow().bold(),
                path.display()
            );
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!(
                "{} could not read {} - ignoring it",
                ">>> Warning:".yellow().bold(),
                path.display()
            );
            continue;
        };
        if parse_into(&text, &path, &mut vars, &mut default_flags) {
            files.push(path);
        }
    }

    let build_vars: Vec<(String, BuildValue)> = BUILD_VARS
        .iter()
        .filter_map(|k| vars.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();

    Config { default_flags, build_vars, files }
}


/// Parses one file into the accumulators. False if it was unusable.
fn parse_into(
    text: &str,
    path: &Path,
    vars: &mut HashMap<String, BuildValue>,
    default_flags: &mut Vec<String>,
) -> bool {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "{} {}: {} - ignoring this file",
                ">>> Warning:".yellow().bold(),
                path.display(),
                e.message()
            );
            return false;
        }
    };

    for (key, value) in &table {
        if key == DEFAULT_FLAG_KEY {
            match as_build_value(value) {
                Some(v) => *default_flags = v.tokens(),
                None => warn(path, key, "expected a string or array of strings"),
            }
            continue;
        }
        if key == "build" {
            match value.as_table() {
                Some(build) => parse_build_table(build, path, vars),
                None => warn(path, key, "expected a [build] table"),
            }
            continue;
        }
        // Tolerate build keys at the top level -- it's the obvious
        // mistake to make, and refusing on a technicality helps nobody.
        if BUILD_VARS.contains(&key.as_str()) {
            match as_build_value(value) {
                Some(v) => {
                    vars.insert(key.clone(), v);
                }
                None => warn(path, key, "expected a string or array of strings"),
            }
            continue;
        }
        warn(path, key, "unknown key (ignored)");
    }
    true
}

fn parse_build_table(build: &toml::Table, path: &Path, vars: &mut HashMap<String, BuildValue>) {
    for (key, value) in build {
        if !BUILD_VARS.contains(&key.as_str()) {
            warn(path, &format!("build.{}", key), "unknown build key (ignored)");
            continue;
        }
        match as_build_value(value) {
            Some(v) => {
                vars.insert(key.clone(), v);
            }
            None => warn(
                path,
                &format!("build.{}", key),
                "expected a string or array of strings",
            ),
        }
    }
}

fn as_build_value(value: &toml::Value) -> Option<BuildValue> {
    match value {
        toml::Value::String(s) => Some(BuildValue::Scalar(s.clone())),
        toml::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(item.as_str()?.to_string());
            }
            Some(BuildValue::List(out))
        }
        _ => None,
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

/// Tokens allowed inside `BUILDENV=()`/`OPTIONS=()`.
fn valid_array_token(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "!_-+.".contains(c))
}

/// makepkg.conf that sources the system one then overrides it with
/// this config's build vars. `None` if none are set (the common case).
pub(crate) fn makepkg_override_conf(cfg: &Config) -> Option<String> {
    if cfg.build_vars.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str("# Generated by aura-emerge from:\n");
    for f in &cfg.files {
        out.push_str(&format!("#   {}\n", f.display()));
    }
    out.push_str("# Do not edit -- rewritten on every build, removed afterwards.\n\n");
    out.push_str(&format!("source {} 2>/dev/null\n\n", crate::MAKEPKG_CONF_SYSTEM));

    let mut exports: Vec<&str> = Vec::new();
    for (key, value) in &cfg.build_vars {
        if ARRAY_VARS.contains(&key.as_str()) {
            let tokens = value.tokens();
            if let Some(bad) = tokens.iter().find(|t| !valid_array_token(t)) {
                eprintln!(
                    "{} emerge.toml: {} contains an unusable entry '{}' - leaving {} as makepkg.conf has it",
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
        out.push_str(&format!("{}=\"{}\"\n", key, esc_double(&value.display())));
        if EXPORT_VARS.contains(&key.as_str()) {
            exports.push(key.as_str());
        }
    }
    if !exports.is_empty() {
        out.push_str(&format!("export {}\n", exports.join(" ")));
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
    ("--only-repos", false),
    ("--skippgp", false),
    ("--autopgp", false),
    ("--no-sandbox", false),
    ("--unshare-net-build", false),
    ("--edit", false),
    ("--skip-srcinfo-regen", false),
    ("--pkgbuild-view", false),
    ("--devel", false),
    ("--keep-going", false),
    ("--sudoloop", false),
    ("--err-install", false),
    ("--refresh", false),
    ("--deep", false),
    ("--newuse", false),
    ("--tree", false),
    ("--columns", false),
    ("--nospinner", false),
    ("--verbose-conflicts", false),
    ("--searchdesc", false),
    ("--skipfirst", false),
    ("--exclude", true),
    ("--color", true),
    ("--jobs", true),
    ("--load-average", true),
    ("--backtrack", true),
    ("--with-bdeps", true),
    ("--quiet-build", true),
];

/// Flag pairs that can't both be in effect, with the reason shown when
/// they are.
const CONFLICTS: &[(&str, &str, &str)] = &[
    ("--aur", "--abs", "they name two different build sources for the same package"),
    ("--aur", "--only-repos", "one forces the AUR, the other forbids it"),
    (
        "--no-sandbox",
        "--unshare-net-build",
        "--unshare-net-build drops the build's network inside the bwrap sandbox, which --no-sandbox turns off entirely",
    ),
    ("--skippgp", "--autopgp", "one skips PGP checks, the other imports keys to satisfy them"),
    ("--select", "--deselect", "one adds to world.set, the other removes from it"),
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
        assert!(parse_into(text, Path::new("test.toml"), &mut vars, &mut flags));
        let build_vars = BUILD_VARS
            .iter()
            .filter_map(|k| vars.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect();
        Config { default_flags: flags, build_vars, files: vec![PathBuf::from("test.toml")] }
    }

    #[test]
    fn default_flags_accept_array_and_string() {
        let a = parse(r#"EMERGE_DEFAULT_OPTS = ["--ask", "--devel"]"#);
        let b = parse(r#"EMERGE_DEFAULT_OPTS = "--ask --devel""#);
        assert_eq!(a.default_flags, vec!["--ask", "--devel"]);
        assert_eq!(a.default_flags, b.default_flags);
    }

    #[test]
    fn build_keys_read_from_table_and_top_level() {
        let nested = parse("[build]\nCFLAGS = \"-O2\"\n");
        let flat = parse("CFLAGS = \"-O2\"\n");
        assert_eq!(nested.build_vars.len(), 1);
        assert_eq!(nested.build_vars[0].1.display(), "-O2");
        assert_eq!(flat.build_vars[0].1.display(), "-O2");
    }

    #[test]
    fn scalars_and_lists_are_interchangeable() {
        let as_list = parse(r#"[build]
OPTIONS = ["strip", "!debug"]"#);
        let as_string = parse(r#"[build]
OPTIONS = "strip !debug""#);
        assert_eq!(as_list.build_vars[0].1.display(), "strip !debug");
        assert_eq!(as_list.build_vars[0].1.tokens(), as_string.build_vars[0].1.tokens());
    }

    #[test]
    fn unknown_keys_are_ignored_not_fatal() {
        let cfg = parse("NONSENSE = \"x\"\n[build]\nALSO_NONSENSE = \"y\"\nCFLAGS = \"-O2\"\n");
        assert_eq!(cfg.build_vars.len(), 1);
        assert_eq!(cfg.build_vars[0].0, "CFLAGS");
    }

    #[test]
    fn malformed_toml_is_reported_not_panicked() {
        let mut vars = HashMap::new();
        let mut flags = Vec::new();
        assert!(!parse_into("CFLAGS = [unclosed", Path::new("t.toml"), &mut vars, &mut flags));
        assert!(vars.is_empty());
    }

    #[test]
    fn generated_conf_sources_system_then_overrides() {
        let cfg = parse("[build]\nCFLAGS = \"-O2\"\nMAKEFLAGS = \"-j$(nproc)\"\nOPTIONS = [\"strip\", \"!debug\"]\nNINJAFLAGS = \"-j4\"\n");
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
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--ask"]"#);
        assert!(makepkg_override_conf(&cfg).is_none());
    }

    #[test]
    fn cli_flag_wins_over_conflicting_config_default() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--aur"]"#);
        let argv = vec!["emerge".to_string(), "--abs".to_string(), "nano".to_string()];
        let out = build_argv(&argv, &cfg);
        assert!(!out.iter().any(|t| t == "--aur"));
        assert!(out.iter().any(|t| t == "--abs"));
    }

    #[test]
    fn config_defaults_precede_the_command_line() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--pkgbuild-view"]"#);
        let argv = vec!["emerge".to_string(), "nano".to_string()];
        assert_eq!(build_argv(&argv, &cfg), vec!["emerge", "--pkgbuild-view", "nano"]);
    }

    #[test]
    fn ignore_default_opts_drops_them_all() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--pkgbuild-view"]"#);
        let argv = vec!["emerge".to_string(), "--ignore-default-opts".to_string()];
        assert_eq!(build_argv(&argv, &cfg), argv);
    }

    #[test]
    fn action_flags_are_not_accepted_as_defaults() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--unmerge", "--ask"]"#);
        let out = build_argv(&vec!["emerge".to_string()], &cfg);
        assert!(!out.iter().any(|t| t == "--unmerge"));
        assert!(out.iter().any(|t| t == "--ask"));
    }

    #[test]
    fn valued_default_flag_keeps_its_value() {
        let cfg = parse(r#"EMERGE_DEFAULT_OPTS = ["--exclude", "linux"]"#);
        let out = build_argv(&vec!["emerge".to_string(), "-u".to_string()], &cfg);
        assert_eq!(out, vec!["emerge", "--exclude", "linux", "-u"]);
    }

    #[test]
    fn abs_and_only_repos_are_not_a_conflict() {
        assert!(find_conflict(&["--abs".to_string(), "--only-repos".to_string()]).is_none());
    }
}