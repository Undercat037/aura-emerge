//! Unit tests for `config` (kept out of the module file so the code stays readable).

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
    Config {
        default_flags: flags,
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
    let cfg = parse(
        "CFLAGS=\"-O2\"\nMAKEFLAGS=\"-j$(nproc)\"\nOPTIONS=(strip !debug)\nNINJAFLAGS=\"-j4\"\n",
    );
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
fn abs_and_only_repos_are_not_a_conflict() {
    assert!(find_conflict(&["--abs".to_string(), "--only-repos".to_string()]).is_none());
}
