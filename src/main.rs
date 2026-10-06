/*
Copyright (C) 2026 Undercat037
This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, version 3 of the License

aura-emerge: A standalone Gentoo-style emerge package manager
for Arch Linux - installs from official repos, the AUR, and ABS;
scans PKGBUILDs for supply-chain attack patterns before building;
and runs untrusted build steps inside a bwrap sandbox.
*/

mod alpm_db;
mod aur;
mod bash_ast;
mod candy;
mod config;
mod helper;
mod logbook;
mod mask;
mod news;
mod package_env;
mod packages;
mod progress;
mod revdep;
mod rootops;
mod runtime;
mod sandbox;
mod security;
mod world_set;

/// Shared blocking HTTP GET (replaces curl subprocesses). None on any failure.
mod http {
    use std::time::Duration;

    /// GET body as text, or None (non-2xx / timeout / network).
    pub(crate) fn get(url: &str, timeout_secs: u64) -> Option<String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(timeout_secs)))
            .build()
            .into();
        agent.get(url).call().ok()?.body_mut().read_to_string().ok()
    }
}

use clap::Parser;
use clap_complete::Shell;
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

use packages::*;
use security::*;
use world_set::*;

// ── Binary paths ───────────────────────────────────────────────────────

pub(crate) const PACMAN_BIN: &str = "/usr/bin/pacman";
pub(crate) const SUDO_BIN: &str = "/usr/bin/sudo";
pub(crate) const TEE_BIN: &str = "/usr/bin/tee";
pub(crate) const MV_BIN: &str = "/usr/bin/mv";
pub(crate) const RM_BIN: &str = "/usr/bin/rm";
pub(crate) const MAKEPKG_BIN: &str = "/usr/bin/makepkg";
pub(crate) const PKGCTL_BIN: &str = "/usr/bin/pkgctl";
pub(crate) const GPG_BIN: &str = "/usr/bin/gpg";
pub(crate) const PGP_KEYSERVER: &str = "keyserver.ubuntu.com";
pub(crate) const ABS_GITLAB_BASE: &str =
    "https://gitlab.archlinux.org/archlinux/packaging/packages";

// ── Files ─────────────────────────────────────────────────────────────────────

pub(crate) const WORLD_SET_FILE: &str = "/etc/portage/world";
/// Custom sets: /etc/portage/sets/<name> or <name>.set → `@<name>`.
pub(crate) const SETS_DIR: &str = "/etc/portage/sets";
// AUR/ABS build roots: packages::aur_build_base()/abs_build_base().
pub(crate) const WORLD_SET_TMP: &str = "/etc/portage/world.tmp";
/// Never-install list; see `mask.rs`.
pub(crate) const MASK_FILE: &str = mask::MASK_FILE;
pub(crate) const RESUME_FILE: &str = "/etc/portage/resume.state";
pub(crate) const RESUME_TMP: &str = "/etc/portage/resume.state.tmp";
pub(crate) const LASTACTION_FILE: &str = "/etc/portage/lastaction.state";
pub(crate) const LASTACTION_TMP: &str = "/etc/portage/lastaction.state.tmp";
pub(crate) const PACMAN_CONF: &str = "/etc/pacman.conf";
pub(crate) const MAKEPKG_CONF_SYSTEM: &str = "/etc/makepkg.conf";
pub(crate) const BASH_BIN: &str = "/usr/bin/bash";
pub(crate) const UNAME_BIN: &str = "/usr/bin/uname";

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Longer DESCRIPTION section for the man page (clap_mangen) - the plain
/// `///` doc comment on `Cli` below is used for the one-line NAME/about.
const LONG_ABOUT: &str = "\
aura-emerge is a standalone Gentoo-style emerge front end for Arch Linux: it drives \
libalpm directly for official-repo packages, and builds AUR and ABS packages itself \
(git clone/checkout, a PKGBUILD supply-chain scan, then a bwrap-sandboxed build) \
rather than shelling out to another AUR helper. \
It tracks every explicitly requested package in /etc/portage/world, independent \
of whatever pacman's own dependency graph currently looks like.\n\n\
Three operations that look similar are kept deliberately distinct: refreshing the \
package databases (--sync), upgrading everything already installed (-u / -u @world), \
and making sure this machine actually has everything world says it should \
(bare @world).";

/// EXTRA section appended after OPTIONS in the man page - worked examples and
/// the @world / custom-sets explanation that doesn't fit as a single flag's help.
const AFTER_HELP: &str = "\
WORLD.SET AND @world
    Every package installed through the normal install path is recorded in
    world as repo/name (e.g. extra/firefox, aur/ayugram-desktop-bin,
    abs/nano-custom). @world has two distinct meanings:

    emerge @world (bare, no -u) -- provision. Installs whatever world
    lists that isn't already on this system; nothing already installed is
    touched. Point world at a synced dotfiles repo and run this on a
    freshly installed machine to pull in the whole package set in one shot.

    emerge -u @world (or plain emerge -u) -- upgrade. Upgrades everything
    already installed (official repos + AUR). Does not read world.

    emerge --sync -- just refreshes the databases, independent of the above.

CUSTOM SETS
    A file /etc/portage/sets/<name> or <name>.set (one package atom per
    line, '#' comments allowed; having both is an error) is invoked as
    @<name>, and can be combined with other
    sets and plain package names in the same command. --list-sets prints
    every set currently available. --regen-sets @<name> re-resolves and
    rewrites the repository prefix on every entry in that set file (same
    idea as --regen-world, but for a custom set instead of world).
    By default this only patches prefixes in place -- line order, '#'
    comments, and blank-line grouping are all preserved. Add --regen-sort
    to also alphabetically re-sort the file (this drops comments and
    blank-line grouping, since a sorted flat list can't keep them
    meaningfully attached to anything). --regen-sort is a modifier, not a
    standalone action -- it only does something alongside --regen-sets.

UNRESOLVED (Err/) PACKAGES AND --err-install
    An entry can end up in world (or get written back by --regen-world
    / --regen-sets) as Err/<name> when it's installed locally but its
    source repo couldn't be determined, or as a bare <name> with no prefix
    at all when it isn't installed and no repo could be found for it
    either. During provisioning (emerge @world / emerge @<name>), a bare
    <name> entry is always resolved the normal way (official repos first,
    AUR on a miss) since there's nothing to lose by trying. An Err/<name>
    entry is more suspect -- it was already installed from *somewhere*
    unknown -- so by default it's only listed, not touched. Pass --err-install
    to have those installed the normal way too.

NEWS
    emerge --news fetches https://archlinux.org/feeds/news/ and lists the
    most recent items, marking unread ones. emerge --news <N> shows item N
    in full and marks it read; emerge --news all dismisses everything
    currently listed. Read/unread state lives in
    ~/.cache/aura-emerge/news.state (per-user, no root required).

EXAMPLES
    emerge neovim                   Install (official repos, falls back to AUR)
    emerge neovim-git --aur         Install explicitly from the AUR (the
                                     named package(s) plus any AUR-only
                                     dependencies, resolved recursively)
    emerge -apt neovim-git --aur    Preview as a dependency tree before
                                     building (-p exits before anything
                                     installs; -a is just muscle memory
                                     here). Add --deep to nest beyond
                                     direct dependencies
    emerge ayugram-desktop-bin --pkgbuild-view  Review the PKGBUILD (diff
                                     on rebuilds) and confirm before it's
                                     built
    emerge ayugram-desktop-bin --scan  Audit the PKGBUILD/.install only -
                                     no build, no install, exits non-zero
                                     on any finding
    emerge --install-pkgbuild ./pkg  Build+install a local PKGBUILD checkout
                                     through the normal scanner+sandbox path
    emerge --batchinstall list.txt  Install every package atom listed in
                                     list.txt, one per line
    emerge -u --exclude linux --keep-going  Upgrade everything except one
                                     package, without stopping the batch
                                     on the first build failure
    emerge --revdep-rebuild         Find + fix binaries linking against a
                                     library that no longer exists
    emerge -u --devel               Upgrade, and also rebuild installed
                                     -git/-hg/-svn/-bzr packages whose
                                     upstream has moved
    emerge --check-devel            Report which -git/-hg/-svn/-bzr
                                     packages are behind upstream, without
                                     rebuilding anything
    emerge @world                   Provision this machine from world
    emerge -u @world                Upgrade the whole system
    emerge @game-kit                Install a custom set
    emerge --prune                  Remove anything not tracked in world
    emerge --news                   List Arch Linux news, newest first
    emerge --news 3                 Read news item 3 in full
    emerge --news all               Dismiss all news notifications

FILES
    /etc/portage/world                      Explicitly-installed packages
    /etc/portage/sets/*                     Custom package sets (<name> or <name>.set)
    /etc/portage/make.conf                  Default flags (EMERGE_DEFAULT_OPTS) and
                                            build-env overrides (CFLAGS, MAKEFLAGS, ...);
                                            system path only (no per-user file)
    /etc/portage/package.mask               Never-install list (file, or a directory of files)
    /etc/portage/package.env                Per-package build-env overrides: `atom env...`
                                            (file, or a directory of files)
    /etc/portage/env/<name>                 Env files for package.env, make.conf syntax
    /etc/portage/resume.state                Saved state for --resume
    /etc/portage/lastaction.state            Last install/unmerge step, for --undo
    /var/log/emerge.log                     Append-only merge/unmerge event log, with
                                            build time for AUR/ABS packages; stats
                                            shown in --info
    ~/.cache/aura-emerge/pkgbuild-view/     Last-shown PKGBUILDs (for --pkgbuild-view diffs)
    ~/.cache/aura-emerge/devel.state        Last-checked upstream refs (--devel/--check-devel)
    ~/.cache/aura-emerge/news.state         Read/unread Arch news items
    ~/.cache/aura-emerge/build/aur/         AUR build checkouts (git clone, scanned, then
                                            built inside the bwrap sandbox)
    ~/.cache/aura-emerge/build/abs/         ABS build checkouts, same pipeline as AUR
    ~/.cache/aura-emerge/sources/           Default SRCDEST -- VCS/source cache that
                                            survives build-dir wipes; overridden by a
                                            SRCDEST set in makepkg.conf
    All ~/.cache paths honor $XDG_CACHE_HOME when set.

INSTALLATION
    Built with `cargo build --release`; the resulting binary is installed as
    /usr/bin/emerge, with /usr/bin/portageq symlinked to it so it can also
    answer as the portageq shim.

    Everything below is generated straight from this Cli definition at
    install time (never hand-edited, so none of it can drift from
    --help):
        emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
        emerge --gen-completions zsh | sudo tee /usr/share/zsh/site-functions/_emerge
        emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
        emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null

AUTHOR
    Undercat037 <https://github.com/Undercat037/aura-emerge>";

/// A standalone Gentoo-style emerge package manager for Arch Linux - installs from official repos, the AUR, and ABS; scans PKGBUILDs for supply-chain attack patterns before building; and runs untrusted build steps inside a bwrap sandbox.

#[derive(Parser, Debug)]
#[command(
    name = "emerge",
    bin_name = "emerge",
    version = concat!(env!("CARGO_PKG_VERSION")),
    disable_help_flag = true,
    disable_version_flag = true,
    long_about = LONG_ABOUT,
    after_long_help = AFTER_HELP,
)]
struct Cli {
    /// Show help information
    #[arg(short = 'h', long = "help", action = clap::ArgAction::SetTrue)]
    help: bool,

    /// Show version
    #[arg(short = 'V', long = "version", action = clap::ArgAction::SetTrue)]
    version: bool,

    /// Search for packages
    #[arg(short = 's', long)]
    search: bool,

    /// Sync package database
    #[arg(long)]
    sync: bool,

    /// Force-refresh databases (pacman -Syy)
    #[arg(long)]
    refresh: bool,

    /// Update packages
    #[arg(short = 'u', long)]
    update: bool,

    /// Remove orphans
    #[arg(short = 'c', long = "depclean")]
    depclean: bool,

    /// With -c: list orphans kept only because they are in world
    #[arg(long = "show-protected", requires = "depclean")]
    show_protected: bool,

    /// Remove specific packages
    #[arg(short = 'C', long = "unmerge")]
    unmerge: bool,

    /// Pretend (dry run)
    #[arg(short = 'p', long = "pretend")]
    pretend: bool,

    /// Ask before applying changes
    #[arg(short = 'a', long = "ask")]
    ask: bool,

    /// Keep sudo timestamp warm (refresh every 60s) for long runs
    #[arg(long = "sudoloop")]
    sudoloop: bool,

    /// Install as dependency (skip world)
    #[arg(short = '1', long = "oneshot")]
    oneshot: bool,

    /// Force AUR only
    #[arg(long = "aur")]
    aur: bool,

    /// Audit PKGBUILD/.install only (no build); non-zero on findings
    #[arg(long = "scan")]
    scan: bool,

    /// With `-u`: also rebuild -git/-hg/-svn/-bzr whose upstream moved
    #[arg(long = "devel")]
    devel: bool,

    /// Report devel packages with upstream drift (no rebuild)
    #[arg(long = "check-devel")]
    check_devel: bool,

    /// Official repos only (never AUR)
    #[arg(long = "repos")]
    repos: bool,

    /// Build from ABS source
    #[arg(long = "abs")]
    abs: bool,

    /// Skip PGP checks on ABS builds (--skippgpcheck)
    #[arg(long = "skippgp")]
    skippgp: bool,

    /// Auto-import missing PGP keys before --abs builds
    #[arg(long = "autopgp")]
    autopgp: bool,

    /// Disable bwrap; plain unisolated makepkg
    #[arg(long = "no-sandbox")]
    no_sandbox: bool,

    /// No network during build()/check()/package() (prepare still has net)
    #[arg(long = "unshare-net-build")]
    unshare_net_build: bool,

    /// Edit PKGBUILD in $EDITOR before build (top-level only; regenerates .SRCINFO)
    #[arg(long = "edit")]
    edit: bool,

    /// With --edit: do not regenerate .SRCINFO after save
    #[arg(long = "skip-srcinfo-regen")]
    skip_srcinfo_regen: bool,

    /// Show/diff PKGBUILD and confirm before build (top-level only)
    #[arg(long = "pkgbuild-view")]
    pkgbuild_view: bool,

    /// Build+install local PKGBUILD dir (scanner + bwrap); world Err/ unless -1
    #[arg(long = "install-pkgbuild", value_name = "PATH", value_hint = clap::ValueHint::DirPath)]
    install_pkgbuild: Option<String>,

    /// Verbose / detailed search info (-sv)
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Do not reinstall if present (pacman --needed)
    #[arg(short = 'n', long = "noreplace")]
    noreplace: bool,

    /// Also install optional dependencies of official-repo packages
    #[arg(long = "with-optdeps")]
    with_optdeps: bool,

    // Dummy flags for compatibility
    /// Include installed pkgs with changed USE flags
    #[arg(short = 'N', long = "newuse")]
    newuse: bool,

    /// Reinstall all world pkgs
    #[arg(short = 'e', long = "emptytree")]
    emptytree: bool,

    /// Resume interrupted merge
    #[arg(long = "resume")]
    resume: bool,

    /// Skip first package on resume
    #[arg(long = "skipfirst")]
    skipfirst: bool,

    /// Undo last install/unmerge (one step; -u not reversible)
    #[arg(long = "undo")]
    undo: bool,

    /// Remove packages not in world
    #[arg(long = "prune")]
    prune: bool,

    /// Regenerate package metadata cache
    #[arg(long = "regen")]
    regen: bool,

    /// Re-resolve repo prefixes in world
    #[arg(long = "regen-world")]
    regen_world: bool,

    /// Re-resolve prefixes in a custom set (`@game-kit` or `game-kit`)
    #[arg(long = "regen-sets", value_name = "SET")]
    regen_sets: Option<String>,

    /// With --regen-sets: also sort the file (drops comments/grouping)
    #[arg(long = "regen-sort", requires = "regen_sets")]
    regen_sort: bool,

    /// On @world: also install Err/ (unknown-source) entries
    #[arg(long = "err-install")]
    err_install: bool,

    /// Seed world from pacman -Qeq (one-time migration)
    #[arg(long = "regen-world-from-explicit")]
    regen_world_from_explicit: bool,

    /// Search package descriptions
    #[arg(short = 'S', long = "searchdesc")]
    searchdesc: bool,

    /// With -s: list the same name from every repo (like `pacman -Ss`)
    #[arg(long = "search-all")]
    search_all: bool,

    /// Add to world without installing
    #[arg(long = "select")]
    select: bool,

    /// Remove from world without unmerging
    #[arg(long = "deselect")]
    deselect: bool,

    /// List world entries that are missing or not explicitly installed
    #[arg(long = "check-world")]
    check_world: bool,

    /// Verbose slot/conflict info (informational)
    #[arg(long = "verbose-conflicts")]
    verbose_conflicts: bool,

    /// List available @sets (also for shell completion)
    #[arg(long = "list-sets")]
    list_sets: bool,

    /// Wipe persistent VCS source cache and exit
    #[arg(long = "clean-source-cache")]
    clean_source_cache: bool,

    /// Mass-install from a text list (same format as a custom set)
    #[arg(long = "batchinstall", value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    batchinstall: Option<String>,

    // ── Gentoo compat flags (accepted silently, no-op) ──────────────────────

    // Actions
    #[arg(long = "metadata")]
    metadata: bool,
    #[arg(long = "clean")]
    clean: bool,
    #[arg(long = "config")]
    config: bool,

    // Output control
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
    #[arg(long = "nospinner")]
    nospinner: bool,
    /// Show makepkg/compiler output live (also AE_DEBUG=1). Default hides
    /// it so the Gentoo-style >>> lines stay readable; failed builds still
    /// dump the captured log.
    #[arg(long = "debug")]
    debug: bool,
    /// Write a session log (command, emerge.log lines, full build output)
    /// to PATH — useful for bug reports without `| tee`.
    #[arg(long = "log", value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    log_path: Option<String>,
    /// Easter egg, hidden from --help and completions.
    #[arg(long = "moo", hide = true)]
    moo: bool,
    #[arg(long = "noconfmem")]
    noconfmem: bool,
    #[arg(long = "color")]
    color: Option<String>,
    #[arg(long = "columns")]
    columns: bool,

    /// Ignore EMERGE_DEFAULT_OPTS from make.conf for this run
    #[arg(long = "ignore-default-opts")]
    ignore_default_opts: bool,

    // Dependency / graph control
    #[arg(short = 'O', long = "nodeps")]
    nodeps: bool,
    #[arg(short = 'o', long = "onlydeps")]
    onlydeps: bool,
    #[arg(short = 't', long = "tree")]
    tree: bool,
    /// With -t: recurse N levels (--deep=N), or every level if bare
    #[arg(short = 'D', long = "deep", value_name = "N", num_args = 0..=1, default_missing_value = "0", require_equals = true)]
    deep: Option<u32>,
    #[arg(long = "complete-graph")]
    complete_graph: bool,
    #[arg(long = "changed-use")]
    changed_use: bool,
    #[arg(long = "backtrack")]
    backtrack: Option<u32>,
    /// Max parallel official-repo installs (default 1)
    #[arg(long = "jobsr", value_name = "N")]
    jobsr: Option<u32>,
    /// Max parallel AUR/ABS builds (default 1)
    #[arg(long = "jobsa", value_name = "N")]
    jobsa: Option<u32>,
    /// Alias for --jobsr (Portage-style)
    #[arg(long = "jobs", value_name = "N")]
    jobs: Option<u32>,
    #[arg(long = "load-average")]
    load_average: Option<f32>,

    /// Don't stop a batch at the first failure; report failures at the end
    #[arg(long = "keep-going")]
    keep_going: bool,

    /// Leave a package out of this run (repeatable, or comma-separated)
    #[arg(long = "exclude", value_name = "ATOM", action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    /// Rebuild packages whose binaries link against a missing library
    #[arg(long = "revdep-rebuild")]
    revdep_rebuild: bool,

    // Binary pkg flags (emerge -k/-K/-g/-G/-b/-B)
    #[arg(short = 'k', long = "usepkg")]
    usepkg: bool,
    #[arg(short = 'K', long = "usepkgonly")]
    usepkgonly: bool,
    #[arg(short = 'g', long = "getbinpkg")]
    getbinpkg: bool,
    #[arg(short = 'G', long = "getbinpkgonly")]
    getbinpkgonly: bool,
    #[arg(short = 'b', long = "buildpkg")]
    buildpkg: bool,
    #[arg(short = 'B', long = "buildpkgonly")]
    buildpkgonly: bool,

    // Fetch flags
    #[arg(short = 'f', long = "fetchonly")]
    fetchonly: bool,
    #[arg(short = 'F', long = "fetch-all-uri")]
    fetch_all_uri: bool,

    // Misc compat
    #[arg(short = 'l', long = "changelog")]
    changelog: bool,
    #[arg(long = "newrepo")]
    newrepo: bool,
    #[arg(long = "reinstall")]
    reinstall: Option<String>,
    #[arg(long = "quiet-build")]
    quiet_build: Option<String>,
    #[arg(long = "with-bdeps")]
    with_bdeps: Option<String>,
    #[arg(long = "alert", short = 'A')]
    alert: bool,

    /// Generate shell completions to stdout; hidden from help.
    #[arg(long = "gen-completions", hide = true, value_name = "SHELL")]
    gen_completions: Option<Shell>,

    /// Generate the man page to stdout; hidden from help.
    #[arg(long = "gen-manpage", hide = true)]
    gen_manpage: bool,

    /// Display system info, like `emerge --info`.
    #[arg(long = "info")]
    info: bool,

    /// Show Arch news; bare `--news` lists items, `--news N` reads one.
    #[arg(long = "news", value_name = "N|all", num_args = 0..=1, default_missing_value = "")]
    news: Option<String>,

    /// Alias for `--news`.
    #[arg(long = "check-news", value_name = "N|all", num_args = 0..=1, default_missing_value = "")]
    check_news: Option<String>,

    /// Packages to install or @set names.
    packages: Vec<String>,
}

fn print_help() {
    let ver = concat!(env!("CARGO_PKG_VERSION"));
    println!("aura-emerge: command-line interface to the Portage system (Arch Linux)");
    println!("Usage:");
    println!("   emerge [ options ] [ action ] [ package | @set ] [ ... ]");
    println!("   emerge [ options ] [ action ] < @world >");
    println!("   emerge < --sync | --info | --list-sets >");
    println!("   emerge --resume [ --pretend | --ask | --skipfirst ]");
    println!("   emerge --help");
    println!("Options: -[1aCcDehNnpstuVv]");
    println!("          [ --abs                        ] [ --aur        ]");
    println!("          [ --skippgp                    ] [ --autopgp    ]");
    println!("          [ --repos                                  ]");
    println!("          [ --edit                       ] [ --skip-srcinfo-regen ]");
    println!("          [ --pkgbuild-view              ] [ --emptytree  ]");
    println!("          [ --newuse                     ] [ --noreplace  ]");
    println!("          [ --oneshot                    ] [ --pretend    ]");
    println!("          [ --skipfirst                  ] [ --refresh    ]");
    println!("          [ --no-sandbox                 ] [ --unshare-net-build ]");
    println!("          [ --devel                      ] [ --sudoloop   ]");
    println!("          [ --verbose-conflicts          ] [ --with-bdeps ]");
    println!("          [ --err-install                ] [ --regen-sort ]");
    println!("          [ --deep[=N]                   ] [ --keep-going ]");
    println!("          [ --exclude <ATOM>             ] [ --ignore-default-opts ]");
    println!("Actions:  [ --depclean  | --deselect | --prune      | --check-world ]");
    println!("          [ --regen     | --resume   | --search     | --searchdesc  ]");
    println!("          [ --select    | --sync     | --unmerge    | --update      ]");
    println!("          [ --regen-world | --version | --info | --regen-world-from-explicit ]");
    println!("          [ --list-sets | --regen-sets @<name>  | --news [N|all]    ]");
    println!("          [ --check-news [N|all]  | --check-devel | --undo           ]");
    println!("          [ --scan <pkg...>       | --install-pkgbuild <PATH>       ]");
    println!("          [ --batchinstall <FILE> | --clean-source-cache            ]");
    println!("          [ --revdep-rebuild                                        ]");
    println!("Sets:     [ @world | @preserved-rebuild | @<custom-sets>            ]");
    println!();
    println!("Full docs, examples and flag-by-flag details: man emerge");
    println!("README: https://github.com/Undercat037/aura-emerge");
    println!();
    println!("aura-emerge: v{}", ver);
    println!("Author: Undercat037");
}

// ── Shell completion: dynamic @set support ──────────────────────────────
//
// Appends a shell snippet that shells out to `emerge --list-sets` when
// completing a word starting with '@', so `emerge @<TAB>` offers real
// set names (clap_complete alone doesn't know sets/'s contents).

fn print_set_completion_glue(shell: Shell) {
    match shell {
        Shell::Bash => {
            println!(
                "{}",
                r#"
# aura-emerge: dynamic @<set> completion (world, preserved-rebuild, sets/*)
_emerge_with_sets() {
    _emerge
    local cur="${COMP_WORDS[COMP_CWORD]}"
    if [[ "$cur" == @* ]]; then
        local sets
        sets=$(emerge --list-sets 2>/dev/null)
        COMPREPLY=( $(compgen -W "$sets" -- "$cur") )
    fi
}
complete -o bashdefault -o default -F _emerge_with_sets emerge 2>/dev/null \
    || complete -F _emerge_with_sets emerge
"#
            );
        }
        Shell::Zsh => {
            println!(
                "{}",
                r#"
# aura-emerge: dynamic @<set> completion (world, preserved-rebuild, sets/*)
_emerge_with_sets() {
    _emerge "$@"
    if [[ "$PREFIX" == @* ]]; then
        local -a sets
        sets=(${(f)"$(emerge --list-sets 2>/dev/null)"})
        compadd -a sets
    fi
}
compdef _emerge_with_sets emerge
"#
            );
        }
        Shell::Fish => {
            println!(
                "{}",
                r#"
# aura-emerge: dynamic @<set> completion (world, preserved-rebuild, sets/*)
complete -c emerge -f -n 'string match -q "@*" -- (commandline -ct)' -a '(emerge --list-sets 2>/dev/null)'
"#
            );
        }
        _ => {
            // Elvish/PowerShell: no glue, static flag completion only.
        }
    }
}

fn validate_pkg(pkg: &str) -> bool {
    if pkg.starts_with('-') || pkg.contains("..") || pkg.contains("//") {
        return false;
    }
    pkg.chars()
        .all(|c| c.is_alphanumeric() || "@._+-/".contains(c))
}

fn validate_packages(packages: &[String]) -> Vec<String> {
    packages
        .iter()
        .filter(|p| {
            if !validate_pkg(p) {
                eprintln!(">>> Invalid package name (skipped): {}", p);
                false
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

/// Normalizes `--exclude`: repeatable, comma-separated, and compared
/// bare, so `--exclude extra/nano,firefox --exclude aur/foo` is three
/// names. Invalid entries are reported and dropped.
fn collect_excludes(raw: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for entry in raw {
        for token in entry.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if !validate_pkg(token) {
                eprintln!(">>> Warning: invalid --exclude entry (skipped): {}", token);
                continue;
            }
            out.insert(token.split('/').last().unwrap_or(token).to_string());
        }
    }
    out
}

/// `pacman -S` for a whole batch, with the `--keep-going` retry.
/// pacman installs a batch as one transaction, so one unresolvable
/// package means nothing gets installed. Without --keep-going this is
/// just `run_cmd`; with it, failed batches retry one package at a time.
///
/// Returns `(all_ok, landed)`. `landed` is what this call actually
/// installed: everything on success, nothing if the single transaction
/// failed or was declined at the prompt, and only the per-package
/// successes after a `--keep-going` retry. Do NOT infer it from
/// `is_installed()` afterwards -- on a reinstall (`[ebuild  R ]`) every
/// package is already installed, so a declined/failed run would still
/// look like it all landed.
/// Official-repo install via the root helper (libalpm), with
/// `--keep-going` retry of individual packages when the batch fails.
/// `args` is kept for call-site compatibility and is otherwise ignored
/// -- the helper does not speak the pacman CLI.

/// Portage-style unread-news heads-up (non-fatal).
fn maybe_news_banner() {
    if let Some(n) = news::unread_count_quiet() {
        if n > 0 {
            println!(
                " {} {}: {} news item(s) need reading for repository '{}'.",
                "*".yellow().bold(),
                "IMPORTANT".yellow().bold(),
                n,
                "arch".bold()
            );
            println!(
                " {} Use {} to view new items.",
                "*".yellow().bold(),
                "emerge --news".cyan()
            );
            println!();
        }
    }
}

/// One repo line: `>>> name...` while pending, `done` once downloaded.
fn sync_line(name: &str, state: Option<&str>) -> String {
    let label = format!("{}...", name);
    let tail = match state {
        Some("updated") => format!(" {}", "done".green().bold()),
        Some("failed") => format!(" {}", "failed".red().bold()),
        // Pending, or already current: no verdict.
        _ => String::new(),
    };
    format!("{} {}{}", ">>>".green().bold(), label, tail)
}

/// Portage-style db sync through the helper (live, one line per repo in
/// pacman.conf order). `force` = `-Syy`. False (error printed) on failure.
pub(crate) fn sync_dbs(force: bool) -> bool {
    use std::io::{IsTerminal, Write};
    use std::sync::atomic::{AtomicBool, Ordering};

    // One sync per run: the world update syncs for the repo half and the
    // AUR half would otherwise sync again. `--refresh` always runs.
    static SYNCED: AtomicBool = AtomicBool::new(false);
    if !force && SYNCED.load(Ordering::Relaxed) {
        return true;
    }

    println!(
        "{} Syncing package databases{}...",
        ">>>".green().bold(),
        if force { " (force refresh)" } else { "" }
    );
    let names = alpm_db::sync_db_names();
    let tty = std::io::stdout().is_terminal();
    let n = names.len();
    let mut states: Vec<Option<String>> = vec![None; n];
    if tty {
        for name in &names {
            println!("{}", sync_line(name, None));
        }
        let _ = std::io::stdout().flush();
    }

    // Helper reports `sync <repo> <updated|uptodate|failed>` as each db
    // finishes (completion order); the line position stays fixed.
    let mut show = |ev: &str| {
        let mut it = ev.split_whitespace();
        let (Some("sync"), Some(repo), Some(state)) = (it.next(), it.next(), it.next()) else {
            return;
        };
        let Some(i) = names.iter().position(|x| x == repo) else {
            return;
        };
        states[i] = Some(state.to_string());
        if tty {
            // up to the line, rewrite it, back down.
            let up = n - i;
            let mut out = std::io::stdout();
            let _ = write!(
                out,
                "\x1b[{up}A\r\x1b[2K{}\x1b[{up}B\r",
                sync_line(repo, Some(state))
            );
            let _ = out.flush();
        }
    };
    let res = rootops::sync(force, &mut show);

    if !tty {
        // No cursor control: print the final state once, in order.
        for (name, st) in names.iter().zip(&states) {
            println!("{}", sync_line(name, st.as_deref()));
        }
    }
    match res {
        Ok(()) => {
            SYNCED.store(true, Ordering::Relaxed);
            crate::candy::mark_start();
            true
        }
        Err(e) => {
            eprintln!("{} {}", ">>> Error:".red().bold(), e);
            false
        }
    }
}

/// Portage-style `>>> Unmerging (n of m) repo/name-version...` loop: one
/// libalpm removal per package, so every line is real. Unconditional
/// (`-Rdd --nosave`), like `emerge -C`. Returns (all ok, names removed).
fn unmerge_loop(names: &[String]) -> (bool, Vec<String>) {
    let total = names.len();
    let repos = get_pkg_repos_batch(names);
    let mut removed: Vec<String> = Vec::new();
    let mut ok = true;
    for (i, name) in names.iter().enumerate() {
        let bare = name.split('/').last().unwrap_or(name);
        let ver = alpm_db::installed_version(bare).unwrap_or_default();
        let repo = match repos.get(bare) {
            Some(Some(r)) if r != "None" => r.clone(),
            _ => String::new(),
        };
        println!(
            "{} Unmerging ({} of {}) {}...",
            ">>>".green().bold(),
            (i + 1).to_string().yellow().bold(),
            total.to_string().yellow().bold(),
            progress::atom(&repo, bare, &ver).green().bold()
        );
        match rootops::remove(
            helper::validate::RemoveMode::Unmerge,
            std::slice::from_ref(name),
        ) {
            Ok(()) => removed.push(name.clone()),
            Err(e) => {
                eprintln!("{} {}: {}", ">>> Error:".red().bold(), name, e);
                ok = false;
                if !runtime::keep_going() {
                    break;
                }
            }
        }
    }
    (ok, removed)
}

/// libalpm install without progress lines (dependency installs inside
/// AUR builds, revdep, preserved-rebuild). `asdeps` marks only packages
/// that were NOT installed before, so an explicit package is never demoted.
pub(crate) fn alpm_install_quiet(
    names: &[String],
    needed: bool,
    asdeps: bool,
) -> Result<(), String> {
    let fresh: Vec<String> = if asdeps {
        names
            .iter()
            .filter(|n| !alpm_db::is_installed(n))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    rootops::install(names, needed)?;
    if !fresh.is_empty() {
        let _ = rootops::set_reason(false, &fresh);
    }
    Ok(())
}

/// Official-repo install through libalpm with live Installing / Completed
/// lines. Batches of up to `--jobsr` packages share one alpm transaction
/// (shown as concurrent Installing, then Completed for each).
/// `asdeps` = `--oneshot`. Targets may be bare or `repo/name`.
pub(crate) fn repo_install_landed(names: &[String], asdeps: bool) -> (bool, Vec<String>) {
    let needed = runtime::get().noreplace;
    let jobsr = runtime::get().jobsr.max(1) as usize;
    let total = names.len();
    if total == 0 {
        return (true, Vec::new());
    }
    progress::reserve(total);
    let syncd = alpm_db::find_sync_many(names);
    let mut landed = Vec::new();

    // Pre-resolve display atoms + helper targets.
    let items: Vec<(String, String, String)> = names
        .iter()
        .map(|name| {
            let bare = name.split('/').last().unwrap_or(name);
            match syncd.get(bare) {
                Some(p) => {
                    let atom = progress::atom(&p.repo, &p.name, &p.version);
                    let target = if name.contains('/') {
                        name.clone()
                    } else {
                        format!("{}/{}", p.repo, p.name)
                    };
                    (name.clone(), atom, target)
                }
                None => (name.clone(), bare.to_string(), name.clone()),
            }
        })
        .collect();

    for chunk in items.chunks(jobsr) {
        // Mark the whole wave as Installing first.
        let mut wave: Vec<(usize, String, String, String)> = Vec::with_capacity(chunk.len());
        for (name, atom, target) in chunk {
            let n = progress::take();
            progress::line(progress::Stage::Installing, n, atom);
            wave.push((n, name.clone(), atom.clone(), target.clone()));
        }
        let targets: Vec<String> = wave.iter().map(|(_, _, _, t)| t.clone()).collect();
        match alpm_install_quiet(&targets, needed, asdeps) {
            Ok(()) => {
                for (n, name, atom, _) in &wave {
                    landed.push(name.clone());
                    progress::line(progress::Stage::Completed, *n, atom);
                }
            }
            Err(batch_err) => {
                // Batch failed: fall back to one-by-one so keep-going
                // can still land the rest of the wave.
                if jobsr > 1 {
                    eprintln!(
                        "{} batch of {} failed ({}); retrying one at a time",
                        ">>>".yellow().bold(),
                        wave.len(),
                        batch_err
                    );
                }
                // Undo the concurrent RUNNING slots opened above; the
                // one-by-one path will re-open them.
                for (n, name, atom, target) in &wave {
                    // Mark failed-looking complete slot so RUNNING drops;
                    // real install below will emit a fresh Installing if needed.
                    // Actually RUNNING was +1 per Installing; we need Completed
                    // or a manual decrement. Emit Completed only on success.
                    // For retry, decrement by treating as aborted install:
                    let _ = n;
                    match alpm_install_quiet(std::slice::from_ref(target), needed, asdeps) {
                        Ok(()) => {
                            landed.push(name.clone());
                            progress::line(progress::Stage::Completed, *n, atom);
                        }
                        Err(e) => {
                            // Close the running slot without a Completed line.
                            progress::abort_one();
                            eprintln!("{} {}: {}", ">>> Error:".red().bold(), name, e);
                            runtime::record_failure(name, "alpm install failed");
                            if !runtime::keep_going() {
                                return (landed.len() == total, landed);
                            }
                        }
                    }
                }
            }
        }
    }
    let ok = landed.len() == total;
    if ok && runtime::get().with_optdeps {
        install_optdeps(&landed);
    }
    (ok, landed)
}

/// `--with-optdeps`: install missing optdepends of `landed` as deps.
fn install_optdeps(landed: &[String]) {
    let extra = alpm_db::missing_optdeps(landed);
    if extra.is_empty() {
        return;
    }
    println!(
        "{} Optional dependencies: {}",
        ">>>".green().bold(),
        extra.join(", ")
    );
    if let Err(e) = rootops::install(&extra, true) {
        eprintln!("{} optdeps: {}", ">>> Error:".red().bold(), e);
        return;
    }
    // Keep them removable by --depclean.
    let _ = rootops::set_reason(false, &extra);
}

/// Bool-only wrapper for callers that don't care what landed.
pub(crate) fn repo_install(names: &[String]) -> bool {
    repo_install_landed(names, false).0
}

/// `emerge -u abs/nano neovim`: upgrade only the named packages.
/// Official → libalpm install (newer sync version); AUR/ABS → rebuild.
fn upgrade_selected(cli: &Cli, targets: &[String]) -> anyhow::Result<()> {
    crate::candy::calculating_deps_line();
    println!();

    let mut official: Vec<String> = Vec::new();
    let mut aur: Vec<String> = Vec::new();
    let mut abs: Vec<String> = Vec::new();

    for raw in targets {
        let bare = raw.split('/').last().unwrap_or(raw).to_string();
        if raw.starts_with("abs/") || (cli.abs && !raw.contains('/')) {
            abs.push(bare);
            continue;
        }
        if raw.starts_with("aur/") || (cli.aur && !raw.contains('/')) {
            aur.push(bare);
            continue;
        }
        // Keep repo/name so the preferred db is used on install.
        let lookup = if raw.contains('/') && !raw.starts_with("abs/") && !raw.starts_with("aur/") {
            raw.clone()
        } else {
            bare.clone()
        };
        if crate::alpm_db::find_sync_many(std::slice::from_ref(&lookup)).contains_key(&bare) {
            official.push(lookup);
        } else {
            aur.push(bare);
        }
    }

    let upgradeable = crate::alpm_db::upgradeable_detail();
    let mut plan_count = 0usize;
    // Only packages with a real upgrade are installed; up-to-date ones are
    // listed and skipped (emerge -u does not reinstall for fun).
    let mut official_todo: Vec<String> = Vec::new();
    for name in &official {
        if let Some((_, old, newv, repo)) = upgradeable.iter().find(|(n, _, _, _)| n == name) {
            let atom = if repo.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", repo, name)
            };
            println!(
                "[{} {:<4}] {} [{} -> {}]",
                "ebuild".green(),
                "U".yellow().bold(),
                atom.yellow().bold(),
                old,
                newv
            );
            official_todo.push(name.clone());
            plan_count += 1;
        } else if crate::alpm_db::is_installed(name) {
            println!(
                "[{} {:<4}] {} (already up to date)",
                "ebuild".green(),
                "R".cyan().bold(),
                name.green().bold()
            );
        } else {
            println!(
                "[{} {:<4}] {}",
                "ebuild".green(),
                "N".green().bold(),
                name.green().bold()
            );
            official_todo.push(name.clone());
            plan_count += 1;
        }
    }
    for name in &aur {
        println!(
            "[{} {:<4}] {} (AUR rebuild)",
            "ebuild".green(),
            "U".yellow().bold(),
            format!("aur/{}", name).yellow().bold()
        );
        plan_count += 1;
    }
    for name in &abs {
        println!(
            "[{} {:<4}] {} (ABS rebuild)",
            "ebuild".green(),
            "U".yellow().bold(),
            format!("abs/{}", name).yellow().bold()
        );
        plan_count += 1;
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), plan_count);
    println!();

    if plan_count == 0 {
        println!(">>> Nothing to upgrade.");
        return Ok(());
    }
    if cli.pretend {
        return Ok(());
    }
    if !confirm_merge(cli.ask) {
        return Ok(());
    }

    if !official_todo.is_empty() {
        println!(">>> Upgrading official packages...");
        let (ok, _) = repo_install_landed(&official_todo, false);
        if !ok && !cli.keep_going {
            return Ok(());
        }
    }
    if !aur.is_empty() {
        println!(">>> Upgrading AUR packages...");
        scan_aur_pkgbuilds_or_abort(&aur);
        let ok = aur_install(
            &aur,
            false,
            false,
            cli.oneshot,
            cli.skippgp,
            cli.edit,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
            cli.pkgbuild_view,
        );
        if !ok && !cli.keep_going {
            return Ok(());
        }
    }
    if !abs.is_empty() {
        println!(">>> Upgrading ABS packages...");
        let ok = abs_install(
            &abs,
            false,
            false,
            cli.oneshot,
            cli.skippgp,
            cli.edit,
            cli.autopgp,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
            cli.pkgbuild_view,
            true, // plan already shown in upgrade_selected
        );
        if !ok && !cli.keep_going {
            return Ok(());
        }
    }
    println!("{} Jobs complete", ">>>".green().bold());
    Ok(())
}

/// Packages to hold back during `-u`: `--exclude` plus every installed
/// package the (unprefixed) mask covers -- prefixed entries are
/// matched in the AUR half instead, where the source is known.
fn upgrade_ignores() -> Vec<String> {
    let mut names: Vec<String> = runtime::get().exclude.iter().cloned().collect();

    if !mask::masks().is_empty() {
        for name in alpm_db::installed_names() {
            if mask::find(&name, None).is_some() {
                names.push(name);
            }
        }
    }

    names.sort();
    names.dedup();
    names
}

// ── Action priority: first exclusive action on the command line wins ─────
// Options may mix freely. --sync is not exclusive (can fall through).
// --scan with --install-pkgbuild is one action + modifier.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Help,
    Version,
    Info,
    News,
    ListSets,
    CleanSourceCache,
    CheckDevel,
    InstallPkgbuild,
    RevdepRebuild,
    Scan,
    Search,
    Regen,
    RegenWorld,
    RegenSets,
    RegenWorldFromExplicit,
    Prune,
    Resume,
    Undo,
    Select,
    Deselect,
    CheckWorld,
    Update,
    Depclean,
    Unmerge,
}

impl ActionKind {
    /// Name as Portage prints it in "Multiple actions requested".
    fn portage_name(self) -> &'static str {
        match self {
            ActionKind::News => "check-news",
            other => other.label().trim_start_matches("--"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ActionKind::Help => "--help",
            ActionKind::Version => "--version",
            ActionKind::Info => "--info",
            ActionKind::News => "--news",
            ActionKind::ListSets => "--list-sets",
            ActionKind::CleanSourceCache => "--clean-source-cache",
            ActionKind::CheckDevel => "--check-devel",
            ActionKind::InstallPkgbuild => "--install-pkgbuild",
            ActionKind::RevdepRebuild => "--revdep-rebuild",
            ActionKind::Scan => "--scan",
            ActionKind::Search => "--search",
            ActionKind::Regen => "--regen",
            ActionKind::RegenWorld => "--regen-world",
            ActionKind::RegenSets => "--regen-sets",
            ActionKind::RegenWorldFromExplicit => "--regen-world-from-explicit",
            ActionKind::Prune => "--prune",
            ActionKind::Resume => "--resume",
            ActionKind::Undo => "--undo",
            ActionKind::Select => "--select",
            ActionKind::Deselect => "--deselect",
            ActionKind::CheckWorld => "--check-world",
            ActionKind::Update => "--update",
            ActionKind::Depclean => "--depclean",
            ActionKind::Unmerge => "--unmerge",
        }
    }
}

fn action_from_long(token: &str) -> Option<ActionKind> {
    let base = token.split('=').next().unwrap_or(token);
    match base {
        "--help" => Some(ActionKind::Help),
        "--version" => Some(ActionKind::Version),
        "--info" => Some(ActionKind::Info),
        "--news" | "--check-news" => Some(ActionKind::News),
        "--list-sets" => Some(ActionKind::ListSets),
        "--clean-source-cache" => Some(ActionKind::CleanSourceCache),
        "--check-devel" => Some(ActionKind::CheckDevel),
        "--install-pkgbuild" => Some(ActionKind::InstallPkgbuild),
        "--revdep-rebuild" => Some(ActionKind::RevdepRebuild),
        "--scan" => Some(ActionKind::Scan),
        "--search" | "--searchdesc" => Some(ActionKind::Search),
        "--regen" => Some(ActionKind::Regen),
        "--regen-world" => Some(ActionKind::RegenWorld),
        "--regen-sets" => Some(ActionKind::RegenSets),
        "--regen-world-from-explicit" => Some(ActionKind::RegenWorldFromExplicit),
        "--prune" => Some(ActionKind::Prune),
        "--resume" => Some(ActionKind::Resume),
        "--undo" => Some(ActionKind::Undo),
        "--select" => Some(ActionKind::Select),
        "--deselect" => Some(ActionKind::Deselect),
        "--check-world" => Some(ActionKind::CheckWorld),
        "--update" => Some(ActionKind::Update),
        "--depclean" => Some(ActionKind::Depclean),
        "--unmerge" => Some(ActionKind::Unmerge),
        _ => None,
    }
}

fn action_from_short_char(c: char) -> Option<ActionKind> {
    match c {
        'h' => Some(ActionKind::Help),
        'V' => Some(ActionKind::Version),
        's' => Some(ActionKind::Search),
        'S' => Some(ActionKind::Search),
        'u' => Some(ActionKind::Update),
        'c' => Some(ActionKind::Depclean),
        'C' => Some(ActionKind::Unmerge),
        _ => None,
    }
}

fn active_actions(cli: &Cli) -> Vec<ActionKind> {
    let mut out = Vec::new();
    if cli.help {
        out.push(ActionKind::Help);
    }
    if cli.version {
        out.push(ActionKind::Version);
    }
    if cli.info {
        out.push(ActionKind::Info);
    }
    if cli.news.is_some() || cli.check_news.is_some() {
        out.push(ActionKind::News);
    }
    if cli.list_sets {
        out.push(ActionKind::ListSets);
    }
    if cli.clean_source_cache {
        out.push(ActionKind::CleanSourceCache);
    }
    if cli.check_devel {
        out.push(ActionKind::CheckDevel);
    }
    if cli.install_pkgbuild.is_some() {
        out.push(ActionKind::InstallPkgbuild);
    }
    if cli.revdep_rebuild {
        out.push(ActionKind::RevdepRebuild);
    }
    // standalone --scan only; with --install-pkgbuild it is a modifier
    if cli.scan && cli.install_pkgbuild.is_none() {
        out.push(ActionKind::Scan);
    }
    if cli.search || cli.searchdesc {
        out.push(ActionKind::Search);
    }
    if cli.regen {
        out.push(ActionKind::Regen);
    }
    if cli.regen_world {
        out.push(ActionKind::RegenWorld);
    }
    if cli.regen_sets.is_some() {
        out.push(ActionKind::RegenSets);
    }
    if cli.regen_world_from_explicit {
        out.push(ActionKind::RegenWorldFromExplicit);
    }
    if cli.prune {
        out.push(ActionKind::Prune);
    }
    if cli.resume {
        out.push(ActionKind::Resume);
    }
    if cli.undo {
        out.push(ActionKind::Undo);
    }
    if cli.select {
        out.push(ActionKind::Select);
    }
    if cli.deselect {
        out.push(ActionKind::Deselect);
    }
    if cli.check_world {
        out.push(ActionKind::CheckWorld);
    }
    if cli.update {
        out.push(ActionKind::Update);
    }
    if cli.depclean {
        out.push(ActionKind::Depclean);
    }
    if cli.unmerge {
        out.push(ActionKind::Unmerge);
    }
    out
}

/// Active actions in argv order (each once); short clusters are scanned
/// letter by letter. Any not seen in argv follow, in `active` order.
fn actions_in_argv_order(argv: &[String], active: &[ActionKind]) -> Vec<ActionKind> {
    let mut out: Vec<ActionKind> = Vec::new();
    let push = |k: ActionKind, out: &mut Vec<ActionKind>| {
        if active.contains(&k) && !out.contains(&k) {
            out.push(k);
        }
    };
    for token in argv.iter().skip(1) {
        if token == "--" {
            break;
        }
        if token.starts_with("--") {
            if let Some(kind) = action_from_long(token) {
                push(kind, &mut out);
            }
        } else if token.starts_with('-') && token.len() > 1 {
            for c in token.chars().skip(1) {
                if let Some(kind) = action_from_short_char(c) {
                    push(kind, &mut out);
                }
            }
        }
    }
    for &k in active {
        push(k, &mut out);
    }
    out
}

/// Portage's text, byte for byte (blank line before and after).
fn multiple_actions_message(a: ActionKind, b: ActionKind) -> String {
    format!(
        "\n!!! Multiple actions requested... Please choose one only.\n!!! '{}' or '{}'\n\n",
        a.portage_name(),
        b.portage_name()
    )
}

/// Like Portage: two actions at once is an error, nothing is guessed.
/// Names the first two in argv order and exits 1.
fn enforce_action_priority(cli: &Cli, argv: &[String]) {
    let order = actions_in_argv_order(argv, &active_actions(cli));
    if let [a, b, ..] = order[..] {
        eprint!("{}", multiple_actions_message(a, b));
        std::process::exit(1);
    }
}

/// After a partially failed `--keep-going` run, replaces the saved
/// resume state with just the failed packages. False (state untouched)
/// when nothing failed.
fn save_failed_resume(cli: &Cli) -> bool {
    let failed = runtime::failed_atoms();
    if failed.is_empty() {
        return false;
    }
    let bare: Vec<String> = failed
        .iter()
        .map(|a| a.split('/').last().unwrap_or(a).to_string())
        .collect();
    save_resume_state(&build_resume_args(cli, &bare, false));
    println!(
        "{} {} package(s) failed; `{}` will retry just those.",
        ">>>".yellow().bold(),
        bare.len(),
        "emerge --resume".cyan()
    );
    true
}

/// Reconstructs the argv-equivalent of the current invocation for
/// `--resume`. Excludes --pretend/--ask/--resume/--skipfirst (how to
/// run it, not what it is).
fn build_resume_args(cli: &Cli, target_pkgs: &[String], has_world: bool) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if cli.update {
        args.push("--update".to_string());
    }
    if cli.aur {
        args.push("--aur".to_string());
    }
    if cli.repos {
        args.push("--repos".to_string());
    }
    if cli.abs {
        args.push("--abs".to_string());
    }
    if cli.skippgp {
        args.push("--skippgp".to_string());
    }
    if cli.autopgp {
        args.push("--autopgp".to_string());
    }
    if cli.edit {
        args.push("--edit".to_string());
    }
    if cli.skip_srcinfo_regen {
        args.push("--skip-srcinfo-regen".to_string());
    }
    if cli.oneshot {
        args.push("--oneshot".to_string());
    }
    if cli.noreplace {
        args.push("--noreplace".to_string());
    }
    if cli.with_optdeps {
        args.push("--with-optdeps".to_string());
    }
    if cli.verbose {
        args.push("--verbose".to_string());
    }
    if cli.refresh {
        args.push("--refresh".to_string());
    }
    if cli.err_install {
        args.push("--err-install".to_string());
    }
    if cli.no_sandbox {
        args.push("--no-sandbox".to_string());
    }
    if cli.unshare_net_build {
        args.push("--unshare-net-build".to_string());
    }
    if cli.keep_going {
        args.push("--keep-going".to_string());
    }
    for e in &cli.exclude {
        args.push("--exclude".to_string());
        args.push(e.clone());
    }
    if has_world {
        args.push("@world".to_string());
    }
    args.extend(target_pkgs.iter().cloned());
    args
}

// ── --sudoloop: keep the sudo timestamp cache warm ─────────────────────────

/// Primes sudo synchronously, then refreshes the timestamp every 60s in
/// a background thread for the process's life. Fire-and-forget.
///
/// Returns false if the initial `sudo -v` fails, so the caller can warn
/// instead of pretending the loop is active.
fn start_sudoloop() -> bool {
    let primed = Command::new(SUDO_BIN)
        .arg("-v")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !primed {
        return false;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let _ = Command::new(SUDO_BIN)
            .arg("-v")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
    true
}

// ── Binary existence check ────────────────────────────────────────────────────

/// Abort early if required binaries are missing.
fn check_binaries() {
    // git is load-bearing since AUR interaction goes through aur.rs
    // directly now. curl is gone from this list -- HTTP now goes
    // through the in-process ureq client in http.rs.
    for bin in &[SUDO_BIN, TEE_BIN, MV_BIN, RM_BIN, aur::GIT_BIN] {
        if !std::path::Path::new(bin).exists() {
            eprintln!(">>> Fatal: required binary not found: {}", bin);
            std::process::exit(1);
        }
    }
}

// ── Symlink guard ─────────────────────────────────────────────────────────────

/// Returns true if the path is safe (not a symlink, or does not exist yet).
/// Prefer `read_to_string_nofollow` / `open_nofollow` for reads — those close
/// the TOCTOU window between this check and the open. Keep this for write-side
/// prechecks and non-file probes.
pub(crate) fn is_safe_path(path: &str) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => !meta.file_type().is_symlink(),
        Err(_) => true,
    }
}

/// Open `path` for reading with `O_NOFOLLOW` so a leaf symlink cannot be
/// swapped in between a metadata check and the open (classic TOCTOU).
/// Parent components may still be symlinks — same model as `is_safe_path`.
#[cfg(unix)]
pub(crate) fn open_nofollow(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
pub(crate) fn open_nofollow(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Like `open_nofollow`, but read+write for in-place edits of existing files.
#[cfg(unix)]
pub(crate) fn open_nofollow_rw(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
pub(crate) fn open_nofollow_rw(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

/// Read the whole file via `open_nofollow`.
pub(crate) fn read_to_string_nofollow(
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = open_nofollow(path.as_ref())?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// True when `err` is the kernel refusing a leaf symlink (`ELOOP`) or a
/// similar "not a regular openable file" failure after `O_NOFOLLOW`.
pub(crate) fn is_symlink_open_error(err: &std::io::Error) -> bool {
    // Linux returns ELOOP when O_NOFOLLOW hits a leaf symlink.
    err.raw_os_error() == Some(libc::ELOOP)
}

// ── AUR search/info output ──────────────────────────────────────────────────

/// `pacman -Ss`-style two-line-per-result listing, for AUR RPC `search`
/// results - replaces parsing/forwarding `aura -As`/`aura --searchdesc`
/// (AUR half) output.

fn print_sync_search_results(results: &[crate::alpm_db::AlpmPkg]) {
    if results.is_empty() {
        return;
    }
    for p in results {
        let mut tags = String::new();
        if p.installed {
            tags.push_str(&format!(" {}", "[installed]".cyan()));
        }
        if crate::mask::find(&p.name, Some(p.repo.as_str())).is_some()
            || crate::mask::find(&p.name, None).is_some()
        {
            tags.push_str(&format!(" {}", "[masked]".red().bold()));
        }
        println!(
            "{}/{} {}{}",
            p.repo.magenta().bold(),
            p.name.bold(),
            p.version.green(),
            tags
        );
        if !p.description.is_empty() {
            println!("    {}", p.description);
        }
    }
}

fn print_abs_search_results(results: &[crate::alpm_db::AlpmPkg]) {
    if results.is_empty() {
        println!(">>> No ABS results found.");
        return;
    }
    for p in results {
        let mut tags = String::new();
        if p.installed {
            tags.push_str(&format!(" {}", "[installed]".cyan()));
        }
        if crate::mask::find(&p.name, Some("abs")).is_some()
            || crate::mask::find(&p.name, None).is_some()
        {
            tags.push_str(&format!(" {}", "[masked]".red().bold()));
        }
        println!(
            "{}/{} {}{}",
            "abs".yellow().bold(),
            p.name.bold(),
            p.version.green(),
            tags
        );
        if !p.description.is_empty() {
            println!("    {}", p.description);
        }
        if p.base != p.name {
            println!("    pkgbase: {}", p.base);
        }
    }
}

fn print_sync_info(pkg: &crate::alpm_db::AlpmPkg) {
    println!("{:<15}: {}", "Repository", pkg.repo.magenta().bold());
    println!("{:<15}: {}", "Name", pkg.name.bold());
    println!("{:<15}: {}", "Version", pkg.version.green());
    println!("{:<15}: {}", "Description", pkg.description);
    println!();
}

fn print_aur_search_results(results: &[aur::AurPkgInfo]) {
    if results.is_empty() {
        println!(">>> No AUR results found.");
        return;
    }
    for r in results {
        let ood = if r.out_of_date {
            " [out of date]".red().bold().to_string()
        } else {
            String::new()
        };
        let masked = if crate::mask::find(&r.name, Some("aur")).is_some()
            || crate::mask::find(&r.name, None).is_some()
        {
            format!(" {}", "[masked]".red().bold())
        } else {
            String::new()
        };
        println!(
            "{}/{} {}{}{} ({} votes, {:.2} popularity)",
            "aur".magenta().bold(),
            r.name.bold(),
            r.version.green(),
            ood,
            masked,
            r.num_votes,
            r.popularity
        );
        if !r.description.is_empty() {
            println!("    {}", r.description);
        }
    }
}

/// `pacman -Si`-style field listing, for AUR RPC `info` results -
/// replaces `aura -Ai` output.
fn print_aur_info_results(results: &[aur::AurPkgInfo]) {
    if results.is_empty() {
        println!(">>> No AUR results found.");
        return;
    }
    for r in results {
        println!("{:<15}: {}", "Repository", "aur".magenta().bold());
        println!("{:<15}: {}", "Name", r.name.bold());
        println!("{:<15}: {}", "Package Base", r.pkgbase);
        println!("{:<15}: {}", "Version", r.version.green());
        println!(
            "{:<15}: {}",
            "Maintainer",
            r.maintainer.as_deref().unwrap_or("(orphan)")
        );
        println!("{:<15}: {}", "Votes", r.num_votes);
        println!("{:<15}: {:.2}", "Popularity", r.popularity);
        println!(
            "{:<15}: {}",
            "Out of Date",
            if r.out_of_date {
                "Yes".red().bold().to_string()
            } else {
                "No".to_string()
            }
        );
        println!("{:<15}: {}", "Description", r.description);
        println!();
    }
}

// ── Command helper ────────────────────────────────────────────────────────────

fn run_cmd(prog: &str, args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new(prog);
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }
    match cmd.status() {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!(">>> Execution error ({}): {}", prog, e);
            false
        }
    }
}

/// Reads one line from stdin byte-by-byte via the raw fd, bypassing
/// Rust's `Stdin` (which over-reads its own buffer) -- a spawned child
/// (pacman) inheriting stdin needs its own interactive read right
/// after, and any over-read bytes would be lost to it.
#[cfg(unix)]
pub(crate) fn read_line_raw() -> String {
    use std::os::unix::io::FromRawFd;
    let mut file = unsafe { std::fs::File::from_raw_fd(0) };
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match std::io::Read::read(&mut file, &mut byte) {
            Ok(0) => break, // EOF
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
            }
            Err(_) => break,
        }
    }
    std::mem::forget(file); // don't close fd 0 out from under the process
    String::from_utf8_lossy(&line)
        .trim_end_matches('\r')
        .to_string()
}

#[cfg(not(unix))]
pub(crate) fn read_line_raw() -> String {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok();
    line.trim_end().to_string()
}

/// Reset SIGPIPE to its default disposition (terminate, not panic).
#[cfg(unix)]
fn reset_sigpipe() {
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--ae-service") {
        // Root helper only. Real/effective uid checked inside guard::harden().
        std::process::exit(helper::run());
    }
    if std::env::args().nth(1).as_deref() == Some("--ae-ping") {
        match helper::client::Client::start().and_then(|mut c| c.ping()) {
            Ok(()) => println!("helper ok"),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    // Frontend must not run as real root (even after `sudo emerge ...`).
    // Privilege stays inside `--ae-service` via the helper protocol.
    // SAFETY: geteuid is a pure getter.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!(
            "{} refusing to run as root. Only `{}` may be root.",
            "emerge:".red().bold(),
            "--ae-service".cyan()
        );
        eprintln!("  Run as your normal user; sudo is prompted when the root helper is needed.");
        std::process::exit(1);
    }
    let result = run();

    // --keep-going collects rather than aborts, so the one place that
    // sees the whole run has to be the one that reports it. Paths that
    // abort outright have already printed their own error and exited.
    let had_failures = runtime::print_failure_summary();

    if let Err(e) = result {
        eprintln!("{} {:#}", ">>> Error:".red().bold(), e);
        std::process::exit(1);
    }
    if had_failures {
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    reset_sigpipe();

    // If invoked as "portageq" (symlink), act as the shim
    let argv: Vec<String> = std::env::args().collect();
    let invoked_as = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if invoked_as == "portageq" {
        portageq_shim(&argv);
        return Ok(());
    }

    // make.conf, before clap: EMERGE_DEFAULT_OPTS is spliced into argv
    // ahead of what was typed, so a typed flag always wins; conflicts
    // (--aur/--abs, --skippgp/--autopgp, ...) are rejected here - see
    // config::CONFLICTS.
    crate::candy::mark_start();
    let cfg = config::load();
    let effective_argv = config::build_argv(&argv, &cfg);
    let cli = Cli::parse_from(&effective_argv);
    enforce_action_priority(&cli, &effective_argv);

    // Everything read from deep inside the build path lands here once.
    let _ = cli.nospinner; // accepted for Portage compat, no-op
                           // Live build output: --debug, AE_DEBUG=1, or Gentoo's --quiet-build=n.
                           // --quiet-build=y (or omitted) keeps the default quiet path.
    let quiet_build_off = matches!(
        cli.quiet_build.as_deref().map(|s| s.trim()),
        Some("n" | "N" | "false" | "False" | "0" | "no" | "No")
    );
    let debug_on = cli.debug
        || quiet_build_off
        || std::env::var_os("AE_DEBUG")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
    if let Some(path) = cli.log_path.as_ref() {
        let cmd_line = effective_argv
            .iter()
            .map(|s| {
                if s.contains(char::is_whitespace) {
                    format!("\"{}\"", s)
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        logbook::session_open(std::path::Path::new(path), &cmd_line);
    }
    // --jobsr wins over legacy --jobs; both default to 1.
    let jobsr = cli.jobsr.or(cli.jobs).unwrap_or(1).max(1);
    let jobsa = cli.jobsa.unwrap_or(1).max(1);
    runtime::init(runtime::Runtime {
        config: cfg,
        exclude: collect_excludes(&cli.exclude),
        keep_going: cli.keep_going,
        debug: debug_on,
        noreplace: cli.noreplace,
        with_optdeps: cli.with_optdeps,
        jobsr,
        jobsa,
    });

    if cli.moo {
        candy::moo();
        return Ok(());
    }

    // No pacman/world needed -- just prints a script. Handled
    // before check_binaries() so it works in a clean chroot too.
    if let Some(shell) = cli.gen_completions {
        let mut cmd = <Cli as clap::CommandFactory>::command();
        clap_complete::generate(shell, &mut cmd, "emerge", &mut io::stdout());
        print_set_completion_glue(shell);
        return Ok(());
    }

    // Man page generation: same idea as --gen-completions, but for
    // emerge(1) - rendered straight from the Cli definition via
    // clap_mangen, so it can't drift out of sync with --help.
    if cli.gen_manpage {
        let cmd = <Cli as clap::CommandFactory>::command();
        let man = clap_mangen::Man::new(cmd);
        let mut buffer: Vec<u8> = Vec::new();
        man.render(&mut buffer)?;
        io::stdout().write_all(&buffer)?;
        return Ok(());
    }

    if cli.help {
        print_help();
        return Ok(());
    }

    if cli.version {
        println!("aura-emerge {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.info {
        print_system_info();
        return Ok(());
    }

    // --news / --check-news (full alias, same [N|all] argument): no
    // aura/pacman/sudo needed (read state lives under ~/.cache, not
    // /etc/emerge), so this runs before check_binaries().
    if let Some(arg) = cli.news.as_deref().or(cli.check_news.as_deref()) {
        news::run_news(arg);
        return Ok(());
    }

    // --list-sets: no pacman needed - just enumerates what's on disk.
    // Also what shell completion shells out to for '@' completion.
    if cli.list_sets {
        println!("@world");
        println!("@preserved-rebuild");
        for name in list_custom_sets() {
            println!("@{}", name);
        }
        return Ok(());
    }

    // --clean-source-cache: same idea, no pacman/sudo needed - just
    // removes a directory under $HOME.
    if cli.clean_source_cache {
        match source_cache_dir() {
            Some(dir) if dir.exists() => {
                if std::fs::remove_dir_all(&dir).is_ok() {
                    println!(
                        "{} removed source cache at {}",
                        ">>>".green().bold(),
                        dir.display()
                    );
                } else {
                    eprintln!(
                        "{} could not remove {}",
                        ">>> Error:".red().bold(),
                        dir.display()
                    );
                    std::process::exit(1);
                }
            }
            Some(dir) => println!(
                "{} no source cache to remove ({} doesn't exist).",
                ">>>".green().bold(),
                dir.display()
            ),
            None => {
                eprintln!("{} could not determine $HOME", ">>> Error:".red().bold());
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    check_binaries();

    // --check-devel: report-only, no sudo needed (no install/pacman -S/
    // -U happens here) - runs before --sudoloop for that reason.
    if cli.check_devel {
        check_devel_all();
        return Ok(());
    }

    if cli.sudoloop {
        if !start_sudoloop() {
            eprintln!(">>> Warning: sudo -v failed; continuing without --sudoloop.");
        }
    }

    // --install-pkgbuild <PATH>: a standalone action, same spirit as --news/
    // --list-sets above but *after* check_binaries()/--sudoloop since it
    // ends in a real `pacman -U` and needs sudo. Doesn't mix with named
    // packages/@sets or --aur/--abs (those name something to *resolve*
    // elsewhere; this already points straight at a checkout on disk).
    if let Some(path_str) = &cli.install_pkgbuild {
        if !cli.packages.is_empty() {
            eprintln!(">>> Error: --install-pkgbuild does not take package names or @sets.");
            std::process::exit(1);
        }
        if cli.aur || cli.abs {
            eprintln!(">>> Error: --install-pkgbuild is not compatible with --aur/--abs.");
            std::process::exit(1);
        }
        // Absolute, no trailing slash: bwrap binds this path, and a
        // relative one ("pkg/") becomes a bogus mount point under the
        // read-only root ("Can't create file pkg/: Read-only file system").
        let path = match std::fs::canonicalize(path_str.trim_end_matches('/')) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(">>> Error: cannot open {}: {}", path_str, e);
                std::process::exit(1);
            }
        };

        // --scan: report-only, no build. Checked first since it's a
        // strict subset of the normal --install-pkgbuild flow below (both
        // start with the same scanner).
        if cli.scan {
            return if crate::security::scan_report_local(
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path_str),
                &path,
            ) {
                Ok(())
            } else {
                std::process::exit(1)
            };
        }

        if cli.pretend {
            println!(
                "{} Would scan and build {} (--pretend: not building).",
                ">>>".green().bold(),
                path.display()
            );
            return Ok(());
        }
        return match pkgbuild_local_install(
            &path,
            cli.ask,
            cli.oneshot,
            cli.skippgp,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
            cli.pkgbuild_view,
        ) {
            Some(names) => {
                if !cli.oneshot {
                    mark_asexplicit(&names);
                    // "Err/" - a genuinely local build with no traceable
                    // origin (see world's own doc comment on that
                    // prefix); there's no AUR/ABS source to record here.
                    if let Err(e) = add_to_world_set(&names, Some("Err")) {
                        eprintln!(
                            ">>> Warning: package(s) installed but world was not updated: {:#}",
                            e
                        );
                    }
                }
                println!("{} Installed: {}", ">>>".green().bold(), names.join(", "));
                Ok(())
            }
            None => std::process::exit(1),
        };
    }

    // --aur/--abs/--repos and the other mutually exclusive pairs are
    // rejected up in config::build_argv, before clap ever runs, so the
    // same table covers flags typed here and flags coming from
    // EMERGE_DEFAULT_OPTS. Note what is deliberately *not* in that table:
    // --abs with --repos. ABS builds an official-repo package from
    // its own source, so "never the AUR" and "build it from source" are
    // two answers to two different questions and agree with each other.
    //
    // --abs with a search is likewise not an error. ABS has no index of
    // its own, so the catalog is the Arch repos in the sync dbs
    // (core/extra/multilib) - see alpm_db::search_abs.
    let repos_only_search = cli.repos || cli.abs;

    // Detect @world / world in package list
    let has_world = cli.packages.iter().any(|p| p == "@world");

    // Detect @preserved-rebuild set (Gentoo-flavored trigger for a
    // dependency-completeness check - see preserved_rebuild()).
    let has_preserved_rebuild = cli.packages.iter().any(|p| p == "@preserved-rebuild");

    // Any other "@name" token is a custom set - resolve it against
    // /etc/portage/sets/<name>[.set] (one package atom per line, '#'
    // comments allowed) and fold its contents into the package list, same
    // as if the user had typed every package in the file by hand.
    let mut custom_set_pkgs: Vec<String> = Vec::new();
    for tok in &cli.packages {
        if let Some(name) = tok.strip_prefix('@') {
            if name == "world" || name == "preserved-rebuild" {
                continue;
            }
            if !valid_set_name(name) {
                eprintln!(">>> Error: invalid set name: @{}", name);
                std::process::exit(1);
            }
            match read_custom_set(name) {
                Ok(pkgs) => {
                    if pkgs.is_empty() {
                        eprintln!(
                            ">>> Warning: set @{} is empty ({}/{}[.set])",
                            name, SETS_DIR, name
                        );
                    }
                    custom_set_pkgs.extend(pkgs);
                }
                Err(e) => {
                    eprintln!(">>> Error: {:#}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // Build validated package list (excluding world/preserved-rebuild/custom-set tokens)
    let mut target_pkgs: Vec<String> = validate_packages(
        &cli.packages
            .iter()
            .filter(|p| *p != "@world" && *p != "@preserved-rebuild" && !p.starts_with('@'))
            .cloned()
            .collect::<Vec<_>>(),
    );
    let from_custom_set = !custom_set_pkgs.is_empty();
    target_pkgs.extend(custom_set_pkgs);

    // --batchinstall <FILE>: fold in a one-off package list from an
    // arbitrary path, same format/validation as a custom set (see
    // world_set::read_batch_file). Folded in here, before any action
    // branches below, so it behaves exactly like packages typed by hand.
    if let Some(path) = &cli.batchinstall {
        match read_batch_file(path) {
            Ok(pkgs) => {
                if pkgs.is_empty() {
                    eprintln!(
                        ">>> Warning: batch file is empty (no valid entries): {}",
                        path
                    );
                }
                target_pkgs.extend(pkgs);
            }
            Err(e) => {
                eprintln!(">>> Error: {:#}", e);
                std::process::exit(1);
            }
        }
    }

    // --exclude / mask, applied once here before any action branch.
    // --exclude means "not this run" (dropped quietly); a masked
    // package requested by name is an error, not a filter.
    if !target_pkgs.is_empty() {
        let requested = target_pkgs.len();
        let (kept, dropped) = runtime::split_excluded(&target_pkgs);
        runtime::report_excluded(&dropped);
        target_pkgs = kept;
        if target_pkgs.is_empty() && requested > 0 && !has_world {
            println!(
                "{} Every requested package was excluded - nothing to do.",
                ">>>".green().bold()
            );
            return Ok(());
        }
        // A mask says "never install this", not "never mention it":
        // searching, scanning, unmerging and deselecting only read or
        // remove what's already there, so they pass straight through.
        // (A masked package is exactly the one you may need to -C.)
        let never_installs = cli.search
            || cli.searchdesc
            || cli.scan
            || cli.unmerge
            || cli.deselect
            || cli.check_world;
        if !never_installs && !mask::allow_explicit(&target_pkgs, None) {
            std::process::exit(1);
        }
    }

    // --revdep-rebuild: whole-system check, no package names, standalone
    // like @preserved-rebuild.
    if cli.revdep_rebuild {
        if !target_pkgs.is_empty() {
            eprintln!(">>> Error: --revdep-rebuild does not take package names or @sets.");
            std::process::exit(1);
        }
        let ok = revdep::revdep_rebuild(
            cli.pretend,
            cli.ask,
            cli.skippgp,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
        );
        if !ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    // --scan: report-only PKGBUILD/.install audit, no build, no install.
    // AUR-only for now (fetched via cgit, same source scan_aur_pkgbuilds_or_abort
    // uses before ever cloning anything) - --abs isn't wired up yet since
    // that needs a throwaway `pkgctl repo clone` with no reusable helper
    // to call standalone today (see aura-emerge-tasks.md).
    if cli.scan {
        if cli.abs {
            eprintln!(">>> Error: --scan doesn't support --abs yet; drop --abs or use --pkgbuild-view during a normal --abs install instead.");
            std::process::exit(1);
        }
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: --scan needs at least one package name (or use --install-pkgbuild <PATH> --scan for a local checkout).");
            std::process::exit(1);
        }
        let mut all_clean = true;
        for pkg in &target_pkgs {
            if !crate::security::scan_report_aur(pkg) {
                all_clean = false;
            }
            println!();
        }
        if !all_clean {
            std::process::exit(1);
        }
        return Ok(());
    }

    // -s doesn't apply to these action sets
    if (cli.search || cli.searchdesc) && (has_world || has_preserved_rebuild) {
        eprintln!(">>> Error: @world / @preserved-rebuild can't be searched.");
        std::process::exit(1);
    }

    // 0. @preserved-rebuild: a standalone action, checked before search/
    //    install so `emerge @preserved-rebuild` (with no other packages)
    //    just runs the check-and-offer-to-fix flow.
    if has_preserved_rebuild {
        preserved_rebuild(cli.pretend, cli.ask);
        return Ok(());
    }

    // 1. Search (including --searchdesc)
    if cli.search || cli.searchdesc {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: Specify search term.");
            std::process::exit(1);
        }

        // Prefix routing for search terms:
        //   aur/nano              → AUR only, term "nano"
        //   abs/nano              → ABS catalog (core/extra/multilib)
        //   cachyos-core-v3/nano  → repos, prefer that repo, term "nano"
        //   nano                  → repos then AUR (unless --aur/--abs/--repos)
        let mut force_aur = cli.aur;
        let mut force_repos = cli.abs || cli.repos;
        let mut force_abs = cli.abs;
        let mut preferred_repo: Option<String> = None;
        let search_pkgs: Vec<String> = target_pkgs
            .iter()
            .map(|p| {
                if let Some(rest) = p.strip_prefix("aur/") {
                    force_aur = true;
                    return rest.to_string();
                }
                if let Some(rest) = p.strip_prefix("abs/") {
                    force_repos = true;
                    force_abs = true;
                    return rest.to_string();
                }
                if let Some((repo, name)) = p.split_once('/') {
                    if !repo.is_empty()
                        && !name.is_empty()
                        && repo
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        preferred_repo = Some(repo.to_string());
                        force_repos = true; // repo/name is official, not AUR
                        return name.to_string();
                    }
                }
                p.clone()
            })
            .collect();
        let term = search_pkgs.join(" ");
        let target_pkgs = search_pkgs;
        let skip_aur = force_repos || repos_only_search;
        let only_aur = force_aur && !force_repos;

        // @set / multi-atom: exact resolve each name (not one fuzzy join).
        // emerge -s @fonts →
        //   noto-fonts ... extra
        //   ttf-comic-sans ... aur
        //   missing-pkg ... not found
        let from_set = from_custom_set;
        let multi_exact = target_pkgs.len() > 1 || from_set;
        if multi_exact && !cli.searchdesc {
            // Batch AUR info once for names not in sync dbs.
            let sync_map = alpm_db::find_sync_many(&target_pkgs);
            let missing_sync: Vec<String> = target_pkgs
                .iter()
                .filter(|n| {
                    let bare = n.split('/').last().unwrap_or(n);
                    !sync_map.contains_key(bare)
                })
                .map(|n| n.split('/').last().unwrap_or(n).to_string())
                .collect();
            let aur_map: HashMap<String, aur::AurPkgInfo> = if only_aur {
                aur::rpc_info(&target_pkgs)
                    .into_iter()
                    .map(|p| (p.name.clone(), p))
                    .collect()
            } else if skip_aur || missing_sync.is_empty() {
                HashMap::new()
            } else {
                aur::rpc_info(&missing_sync)
                    .into_iter()
                    .map(|p| (p.name.clone(), p))
                    .collect()
            };

            let width = target_pkgs
                .iter()
                .map(|n| n.split('/').last().unwrap_or(n).len())
                .max()
                .unwrap_or(8)
                .max(8);

            for raw in &target_pkgs {
                let bare = raw.split('/').last().unwrap_or(raw);
                if only_aur {
                    if let Some(p) = aur_map.get(bare) {
                        println!(
                            "{:<width$}  {}  {}",
                            bare,
                            "aur".cyan(),
                            p.version.dimmed(),
                            width = width
                        );
                    } else {
                        println!(
                            "{:<width$}  {}",
                            bare,
                            "not found".red().bold(),
                            width = width
                        );
                    }
                    continue;
                }
                if let Some(p) = sync_map.get(bare) {
                    println!(
                        "{:<width$}  {}  {}",
                        bare,
                        p.repo.green(),
                        p.version.dimmed(),
                        width = width
                    );
                } else if !skip_aur {
                    if let Some(p) = aur_map.get(bare) {
                        println!(
                            "{:<width$}  {}  {}",
                            bare,
                            "aur".cyan(),
                            p.version.dimmed(),
                            width = width
                        );
                    } else {
                        println!(
                            "{:<width$}  {}",
                            bare,
                            "not found".red().bold(),
                            width = width
                        );
                    }
                } else {
                    println!(
                        "{:<width$}  {}",
                        bare,
                        "not found".red().bold(),
                        width = width
                    );
                }
            }
            return Ok(());
        }

        // Rank: exact name match first, then preferred repo, then the rest.
        let rank_sync = |pkgs: &mut Vec<crate::alpm_db::AlpmPkg>| {
            pkgs.sort_by(|a, b| {
                let ae = a.name.eq_ignore_ascii_case(&term);
                let be = b.name.eq_ignore_ascii_case(&term);
                match (ae, be) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => {
                        if let Some(ref pref) = preferred_repo {
                            let ar = a.repo == *pref;
                            let br = b.repo == *pref;
                            match (ar, br) {
                                (true, false) => std::cmp::Ordering::Less,
                                (false, true) => std::cmp::Ordering::Greater,
                                _ => a.name.cmp(&b.name),
                            }
                        } else {
                            a.name.cmp(&b.name)
                        }
                    }
                }
            });
        };

        // Separate ABS source in a general search (shown wherever AUR is).
        let abs_section = |term: &str| {
            println!();
            println!(
                "{} Searching in {} for '{}'...",
                ">>>".green().bold(),
                "ABS".yellow().bold(),
                term
            );
            let mut abs = crate::alpm_db::search_abs(term);
            rank_sync(&mut abs);
            print_abs_search_results(&abs);
        };

        if force_abs {
            println!(
                "{} Searching in {} for '{}'...",
                ">>>".green().bold(),
                "ABS".yellow().bold(),
                term
            );
            let mut abs = crate::alpm_db::search_abs(&term);
            rank_sync(&mut abs);
            print_abs_search_results(&abs);
            return Ok(());
        }

        if cli.searchdesc {
            if !only_aur {
                println!(
                    "{} Searching descriptions for '{}'...",
                    ">>>".green().bold(),
                    term
                );
                let mut sync = crate::alpm_db::search_sync(&term, true, cli.search_all);
                rank_sync(&mut sync);
                print_sync_search_results(&sync);
            }
            if !skip_aur {
                abs_section(&term);
            }
            if !skip_aur {
                println!();
                println!(
                    "{} Searching {} descriptions for '{}'...",
                    ">>>".green().bold(),
                    "AUR".cyan().bold(),
                    term
                );
                print_aur_search_results(&aur::rpc_search(&term, true));
            }
            return Ok(());
        }

        if cli.verbose {
            if only_aur {
                println!(
                    "{} Searching in {} for '{}'...",
                    ">>>".green().bold(),
                    "AUR".cyan().bold(),
                    term
                );
                let info = aur::rpc_info(&target_pkgs);
                if !info.is_empty() {
                    print_aur_info_results(&info);
                } else {
                    print_aur_search_results(&aur::rpc_search(&term, false));
                }
            } else {
                let found = probe_official(&target_pkgs).is_some();
                if found {
                    println!("{} Searching for '{}'...", ">>>".green().bold(), term);
                    for name in &target_pkgs {
                        if let Some(p) = crate::alpm_db::find_sync(name) {
                            print_sync_info(&p);
                        }
                    }
                } else {
                    println!("{} Searching for '{}'...", ">>>".green().bold(), term);
                    let mut sync = crate::alpm_db::search_sync(&term, false, cli.search_all);
                    rank_sync(&mut sync);
                    print_sync_search_results(&sync);
                    if !skip_aur {
                        abs_section(&term);
                    }
                    if !skip_aur {
                        println!();
                        println!(
                            "{} Searching in {} for '{}'...",
                            ">>>".green().bold(),
                            "AUR".cyan().bold(),
                            term
                        );
                        print_aur_search_results(&aur::rpc_search(&term, false));
                    } else if preferred_repo.is_some() || force_repos {
                        // repo/name or --abs/--repos: stay quiet if empty
                    }
                }
            }
        } else if only_aur {
            println!(
                "{} Searching in {} for '{}'...",
                ">>>".green().bold(),
                "AUR".cyan().bold(),
                term
            );
            print_aur_search_results(&aur::rpc_search(&term, false));
        } else {
            println!("{} Searching for '{}'...", ">>>".green().bold(), term);
            let mut sync = crate::alpm_db::search_sync(&term, false, cli.search_all);
            rank_sync(&mut sync);
            print_sync_search_results(&sync);
            if !skip_aur {
                abs_section(&term);
            }
            if !skip_aur {
                println!();
                println!(
                    "{} Searching in {} for '{}'...",
                    ">>>".green().bold(),
                    "AUR".cyan().bold(),
                    term
                );
                print_aur_search_results(&aur::rpc_search(&term, false));
            }
        }
        return Ok(());
    }

    // 2. Sync - sync DB, then continue to install if packages given
    if cli.sync || cli.refresh {
        // --sync = -Sy, --refresh = -Syy (via helper, quiet).
        if !sync_dbs(cli.refresh) {
            std::process::exit(1);
        }
        if target_pkgs.is_empty() && !has_world {
            return Ok(());
        }
        // fall through to update or install
    }

    // --regen: regenerate package metadata cache
    if cli.regen {
        println!(
            "{} Regenerating package metadata cache...",
            ">>>".green().bold()
        );
        run_cmd(SUDO_BIN, &[PACMAN_BIN, "-Fy"], &[]);
        return Ok(());
    }

    // --regen-world: re-resolve repo prefixes for all entries in world
    if cli.regen_world {
        regen_world_set()?;
        return Ok(());
    }

    // --regen-sets @<name>: same idea as --regen-world, but for one custom
    // set under /etc/portage/sets/<name>[.set]. Accepts either "@name" or
    // bare "name".
    if let Some(set_arg) = &cli.regen_sets {
        let name = set_arg.strip_prefix('@').unwrap_or(set_arg);
        if !valid_set_name(name) {
            eprintln!(">>> Error: invalid set name: @{}", name);
            std::process::exit(1);
        }
        regen_set(name, cli.regen_sort)?;
        return Ok(());
    }

    // --check-world: world entries missing or not explicit (asdeps).
    if cli.check_world {
        if !is_safe_path(WORLD_SET_FILE) {
            eprintln!(
                ">>> Warning: {} is a symlink - refusing to read",
                WORLD_SET_FILE
            );
            std::process::exit(1);
        }
        let world_lines: Vec<String> = match fs::File::open(WORLD_SET_FILE) {
            Ok(file) => io::BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect(),
            Err(_) => {
                eprintln!(">>> Error: cannot open world");
                std::process::exit(1);
            }
        };
        let explicit = crate::alpm_db::explicit_set();
        let mut missing = Vec::new();
        let mut asdeps = Vec::new();
        for entry in &world_lines {
            let bare = entry.split('/').last().unwrap_or(entry);
            if !crate::alpm_db::is_installed(bare) {
                missing.push(entry.clone());
            } else if !explicit.contains(bare) {
                asdeps.push(entry.clone());
            }
        }
        println!("{} World audit", ">>>".green().bold());
        println!("    entries: {}", world_lines.len());
        println!("    not installed: {}", missing.len());
        println!("    installed asdeps (not explicit): {}", asdeps.len());
        if !missing.is_empty() {
            println!();
            println!("{} not installed:", " *".yellow().bold());
            for e in &missing {
                println!("    {}", e);
            }
        }
        if !asdeps.is_empty() {
            println!();
            println!(
                "{} in world but install reason is dependency:",
                " *".yellow().bold()
            );
            for e in &asdeps {
                println!("    {}", e);
            }
        }
        if missing.is_empty() && asdeps.is_empty() {
            println!(">>> All world entries are installed and explicit.");
        }
        return Ok(());
    }

    // --regen-world-from-explicit: seed world from every currently
    // explicitly-installed package (pacman -Qeq). Meant as a one-time
    // migration step on a system that predates world tracking - run
    // it once, and `-c`'s world protection (below) and `--prune`
    // start seeing the whole system instead of just what was installed
    // through `emerge` since world existed.
    if cli.regen_world_from_explicit {
        println!(
            "{} Seeding world from explicitly installed packages...",
            ">>>".green().bold()
        );
        let explicit = crate::alpm_db::explicit_names();
        if explicit.is_empty() {
            println!(">>> No explicitly installed packages found.");
            return Ok(());
        }
        println!(
            ">>> Found {} explicitly installed package(s). Resolving repo prefixes...",
            explicit.len()
        );
        add_to_world_set(&explicit, None)?;
        return Ok(());
    }

    // --prune: remove installed packages not in world
    if cli.prune {
        println!("{} Pruning packages not in world...", ">>>".green().bold());
        if !is_safe_path(WORLD_SET_FILE) {
            eprintln!(
                ">>> Warning: {} is a symlink - refusing to read",
                WORLD_SET_FILE
            );
            std::process::exit(1);
        }
        let world_bare: HashSet<String> = match fs::File::open(WORLD_SET_FILE) {
            Ok(file) => io::BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().split('/').last().unwrap_or("").to_string())
                .collect(),
            Err(_) => {
                eprintln!(">>> Error: cannot open world");
                std::process::exit(1);
            }
        };
        let installed = crate::alpm_db::explicit_names();
        let to_remove: Vec<String> = installed
            .into_iter()
            .filter(|p| !world_bare.contains(p))
            .collect();
        let (to_remove, excluded) = runtime::split_excluded(&to_remove);
        runtime::report_excluded(&excluded);
        if to_remove.is_empty() {
            println!(">>> Nothing to prune. All explicitly installed packages are in world.");
            return Ok(());
        }
        println!();
        for p in &to_remove {
            println!("[{}] {}", "unmerge".red().bold(), p);
        }
        println!();
        println!("Total: {} package(s) to prune", to_remove.len());
        println!();
        if cli.pretend {
            return Ok(());
        }
        if !confirm_action(cli.ask, "unmerge") {
            return Ok(());
        }
        match rootops::remove(helper::validate::RemoveMode::Prune, &to_remove) {
            Ok(()) => logbook::log_unmerge(&to_remove),
            Err(e) => eprintln!("{} {}", ">>> Error:".red().bold(), e),
        }
        return Ok(());
    }

    // --resume: re-run the last interrupted install/@world operation,
    // exactly as it was invoked (same flags, same packages). Falls back to
    // a plain full-system upgrade if nothing was saved (e.g. first run
    // after upgrading aura-emerge, or the last operation already finished).
    if cli.resume {
        println!(">>> Attempting to resume last interrupted transaction...");
        match load_resume_state() {
            Some(mut args) => {
                if cli.skipfirst {
                    // Package names can never start with '-' (validate_pkg
                    // rejects that), and "@world" is never a real package,
                    // so the first token matching neither is unambiguously
                    // the first package in the resumed list.
                    if let Some(pos) = args
                        .iter()
                        .position(|a| !a.starts_with('-') && a != "@world")
                    {
                        println!(">>> Skipping first package in resume list: {}", args[pos]);
                        args.remove(pos);
                    }
                }
                if cli.pretend && !args.iter().any(|a| a == "--pretend" || a == "-p") {
                    args.push("--pretend".to_string());
                }
                if cli.ask && !args.iter().any(|a| a == "--ask" || a == "-a") {
                    args.push("--ask".to_string());
                }
                println!(
                    "{} emerge {}",
                    ">>> Resuming:".green().bold(),
                    args.join(" ")
                );

                let exe = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/usr/bin/emerge"));
                match Command::new(exe).args(&args).status() {
                    Ok(s) if s.success() => {}
                    Ok(_) => std::process::exit(1),
                    Err(e) => {
                        eprintln!(">>> Error: failed to re-invoke emerge for resume: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            None => {
                println!(">>> No saved transaction to resume.");
                println!(">>> Falling back to a full system upgrade instead.");
                if !confirm_merge(cli.ask) {
                    return Ok(());
                }
                if !sync_dbs(false) {
                    std::process::exit(1);
                }
                if let Err(e) = rootops::sysupgrade(&[], &mut |_| {}) {
                    eprintln!("{} {}", ">>> Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
        return Ok(());
    }

    // --undo: reverse the last successful install or unmerge. Deliberately
    // narrow - see the flag's help text for exactly what is and isn't
    // covered.
    if cli.undo {
        match load_last_action() {
            Some((kind, _atoms)) if kind == "update" => {
                // Full-system upgrades aren't reversible package-by-package
                // (no record of prior versions), and this tool no longer
                // takes its own snapshot before an upgrade (see the update
                // branch below) - that used to be aura's own `-B`
                // state-save, backing `-Br` here. Neither exists anymore,
                // so there's genuinely nothing to restore to.
                eprintln!(
                    "{} Full-system-upgrade undo isn't available - this build no longer takes a \
                    pre-upgrade snapshot. Check `pacman -Qi <pkg>` / the pacman log \
                    (/var/log/pacman.log) and downgrade specific packages manually if needed \
                    (`pacman -U` against a cached .pkg.tar.* in /var/cache/pacman/pkg/).",
                    ">>> Error:".red().bold()
                );
                std::process::exit(1);
            }
            Some((kind, atoms)) if kind == "install" => {
                println!(
                    "{} Undoing last install - removing: {}",
                    ">>>".green().bold(),
                    atoms.join(", ")
                );
                let bare: Vec<String> = atoms
                    .iter()
                    .map(|a| a.split('/').last().unwrap_or(a).to_string())
                    .collect();
                if cli.pretend {
                    return Ok(());
                }
                let success = match rootops::remove(helper::validate::RemoveMode::Plain, &bare) {
                    Ok(()) => true,
                    Err(e) => {
                        eprintln!("{} {}", ">>> Error:".red().bold(), e);
                        false
                    }
                };
                if success {
                    if let Err(e) = remove_from_world_set(&bare) {
                        eprintln!(
                            ">>> Warning: package(s) removed but world was not updated: {:#}",
                            e
                        );
                    }
                    clear_last_action();
                } else {
                    eprintln!(
                        ">>> Warning: undo did not fully succeed - saved state left in place."
                    );
                    std::process::exit(1);
                }
            }
            Some((kind, atoms)) if kind == "unmerge" => {
                println!(
                    "{} Undoing last unmerge - reinstalling: {}",
                    ">>>".green().bold(),
                    atoms.join(", ")
                );
                if cli.pretend {
                    return Ok(());
                }
                let mut all_ok = true;

                let official: Vec<String> = atoms
                    .iter()
                    .filter(|a| {
                        !a.starts_with("aur/") && !a.starts_with("abs/") && !a.starts_with("Err/")
                    })
                    .cloned()
                    .collect();
                let aur: Vec<String> = atoms
                    .iter()
                    .filter(|a| a.starts_with("aur/"))
                    .cloned()
                    .collect();
                let unresolved: Vec<String> = atoms
                    .iter()
                    .filter(|a| a.starts_with("abs/") || a.starts_with("Err/"))
                    .cloned()
                    .collect();

                if !official.is_empty() {
                    let names: Vec<String> = official
                        .iter()
                        .map(|a| a.split('/').last().unwrap_or(a).to_string())
                        .collect();
                    if repo_install_landed(&names, false).0 {
                        mark_asexplicit(&names);
                        if let Err(e) = add_to_world_set(&names, None) {
                            eprintln!(">>> Warning: package(s) reinstalled but world was not updated: {:#}", e);
                        }
                    } else {
                        all_ok = false;
                    }
                }
                if !aur.is_empty() {
                    let names: Vec<String> = aur
                        .iter()
                        .map(|a| a.split('/').last().unwrap_or(a).to_string())
                        .collect();
                    scan_aur_pkgbuilds_or_abort(&names);
                    // aur_install() always leaves the explicit bit set on
                    // success (see its doc comment) - no separate
                    // mark_asexplicit() call needed the way `aura -A` required.
                    if aur_install(
                        &names,
                        false,
                        cli.ask,
                        false,
                        cli.skippgp,
                        cli.edit,
                        cli.no_sandbox,
                        cli.skip_srcinfo_regen,
                        cli.unshare_net_build,
                        false,
                    ) {
                        if let Err(e) = add_to_world_set(&names, Some("aur")) {
                            eprintln!(">>> Warning: package(s) reinstalled but world was not updated: {:#}", e);
                        }
                    } else {
                        all_ok = false;
                    }
                }
                if !unresolved.is_empty() {
                    eprintln!(
                        ">>> Warning: the following package(s) had no known source (abs/Err) and \
                        were not auto-reinstalled - reinstall them manually if needed:"
                    );
                    for u in &unresolved {
                        eprintln!("    {}", u);
                    }
                }

                if all_ok {
                    clear_last_action();
                } else {
                    eprintln!(
                        ">>> Warning: undo did not fully succeed - saved state left in place."
                    );
                    std::process::exit(1);
                }
            }
            Some((kind, _)) => {
                eprintln!(">>> Error: unrecognized saved undo state ({})", kind);
                std::process::exit(1);
            }
            None => {
                println!(
                    "{} Nothing to undo - no saved action found.",
                    ">>>".green().bold()
                );
            }
        }
        return Ok(());
    }

    // --select: explicitly add packages to world without installing
    if cli.select {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: specify packages to add to world.");
            std::process::exit(1);
        }
        for p in &target_pkgs {
            println!(">>> Selecting {} into world...", p);
        }
        add_to_world_set(&target_pkgs, None)?;
        // world now says these are explicitly wanted - mirror that onto
        // pacman's own bookkeeping (no-ops for anything not yet installed).
        mark_asexplicit(&target_pkgs);
        return Ok(());
    }

    // --deselect: remove packages from world without unmerging
    if cli.deselect {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: specify packages to deselect from world.");
            std::process::exit(1);
        }
        for p in &target_pkgs {
            println!(
                ">>> Deselecting {} from world (package stays installed)...",
                p
            );
        }
        remove_from_world_set(&target_pkgs)?;
        // Opposite of --select: no longer wanted by world, so demote to
        // a dependency in pacman's bookkeeping - makes it eligible for
        // --depclean like any other transitive dependency.
        mark_asdeps(&target_pkgs);
        return Ok(());
    }

    // 3a. Mixed: specific pkgs + @world, no -u (e.g. `emerge nano @world`)
    //     Install the named packages first (falls through to the normal
    //     install block below); once that finishes, provision anything
    //     else still missing from world. See `provision_after_install`.
    let provision_after_install = has_world && !target_pkgs.is_empty() && !cli.update;
    if provision_after_install {
        println!(
            "{} Installing specified packages, then provisioning the rest of world...",
            ">>>".green().bold()
        );
    }

    // 3b. Bare `@world`, no other packages, no -u: declarative
    // provisioning - install whatever world lists that isn't already
    // on this system, and touch nothing that already is. This is the
    // "move world to a new machine and get everything back"
    // operation (or "make sure this machine matches what I asked for").
    // For a full system upgrade use `-u @world` / `-u` (below); for just
    // refreshing the databases, `--sync`.
    if has_world && target_pkgs.is_empty() && !cli.update {
        if !cli.pretend {
            save_resume_state(&build_resume_args(&cli, &[], true));
        }
        let ok = provision_from_world_set(
            cli.pretend,
            cli.ask,
            cli.verbose,
            cli.err_install,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
        )?;
        if !cli.pretend {
            if ok {
                clear_resume_state();
            } else {
                eprintln!(
                    "{} not everything installed successfully.",
                    ">>> Warning:".yellow().bold()
                );
                save_failed_resume(&cli);
                if runtime::any_failures() {
                    return Ok(());
                }
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // 3. Full system upgrade - triggered by -u, with or without @world.
    // Equivalent to Gentoo's `emerge -u @world`: upgrades everything
    // already installed (official repos + AUR), it does not consult
    // world at all. For -Syy use `--sync --refresh`.
    // 3. Upgrade: -u alone / -u @world = full system; -u pkg… = only those.
    // Official half via libalpm sysupgrade (or install for selective);
    // AUR/ABS rebuilt when named or during full -u.
    if cli.update {
        // Named packages with -u: selective upgrade, not full system.
        let selective = !target_pkgs.is_empty() && !has_world;
        if selective {
            return upgrade_selected(&cli, &target_pkgs);
        }

        maybe_news_banner();

        if !cli.pretend {
            save_resume_state(&build_resume_args(&cli, &target_pkgs, has_world));
        }

        // No pre-upgrade snapshot is taken here anymore (previously
        // `aura -B`, backing `emerge --undo` for a full-system upgrade -
        // dropped along with the rest of aura; see the --undo "update"
        // branch above for what that means for `--undo` now). The
        // resume-state save above is unrelated and unaffected - `--resume`
        // still works the same way.
        if !cli.pretend {
            save_last_action(LastAction::Update, &["system".to_string()]);
        }

        // Sync first: the plan below (and the "nothing to merge" check)
        // must be computed from fresh dbs, not from last week's. One sync
        // per run (SYNCED), so `--sync -u` does not sync twice.
        if !cli.pretend && !sync_dbs(false) {
            std::process::exit(1);
        }

        crate::candy::calculating_deps_line();
        println!();
        println!(">>> Upgrading system (official repos)...");
        // --exclude and the mask both become `pacman --ignore`.
        let ignores = upgrade_ignores();
        if !ignores.is_empty() {
            println!(
                "{} holding back {} package(s) (--exclude / {}): {}",
                ">>>".yellow().bold(),
                ignores.len(),
                MASK_FILE,
                ignores.join(", ")
            );
        }
        // `-Sy` (refresh) writes to the local sync db and needs root
        // regardless of `--print`. `-Su --print` (upgrade-only, no
        // refresh) reads the already-synced db instead and needs no
        // privilege escalation at all, matching how `--pretend` behaves
        // everywhere else in this tool (never asks for sudo). Real runs
        // still refresh via `-Syu` as before; if the synced db is stale,
        // run `--sync` first for an accurate preview.
        let ignored: HashSet<&str> = ignores.iter().map(String::as_str).collect();
        let official_upgrades: Vec<(String, String, String, String)> =
            crate::alpm_db::upgradeable_detail()
                .into_iter()
                .filter(|(n, _, _, _)| !ignored.contains(n.as_str()))
                .collect();

        // Always show the ebuild-style plan (pretend and real).
        if official_upgrades.is_empty() {
            println!(">>> No official packages out of date.");
        } else {
            println!();
            for (name, old, newv, repo) in &official_upgrades {
                let atom = if repo.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", repo, name)
                };
                println!(
                    "[{} {:<4}] {} [{} -> {}]",
                    "ebuild".green(),
                    "U".yellow().bold(),
                    atom.yellow().bold(),
                    old,
                    newv
                );
            }
            println!();
            println!(
                "{}: {} official package(s) to upgrade",
                "Total".bold(),
                official_upgrades.len()
            );
            println!();
        }

        // AUR half planned up front so repo + AUR share one numbered pool.
        let aur_plan = aur_upgrade_plan(cli.devel);

        // Nothing to do: no prompt, no auto-clean noise.
        if official_upgrades.is_empty() && aur_plan.is_empty() {
            println!();
            println!(">>> Nothing to merge; quitting.");
            if !cli.pretend {
                clear_resume_state();
            }
            return Ok(());
        }

        let ok1 = if cli.pretend {
            true
        } else if !confirm_merge(cli.ask) {
            return Ok(());
        } else if !sync_dbs(false) {
            false
        } else {
            progress::begin(official_upgrades.len() + aur_plan.len());
            let atoms: HashMap<String, String> = official_upgrades
                .iter()
                .map(|(n, _, v, r)| (n.clone(), progress::atom(r, n, v)))
                .collect();
            let mut nums: HashMap<String, usize> = HashMap::new();
            // `plan <n>` from the helper: how many packages libalpm queued.
            let mut tx_size: Option<usize> = None;
            let mut done_pkgs = 0usize;
            // Helper: `pkg start|done <name>` per package, live.
            let mut show = |ev: &str| {
                let mut it = ev.split_whitespace();
                let (Some(head), Some(kind)) = (it.next(), it.next()) else {
                    return;
                };
                if head == "plan" {
                    tx_size = kind.parse().ok();
                    return;
                }
                let (true, Some(name)) = (head == "pkg", it.next()) else {
                    return;
                };
                let atom = atoms.get(name).cloned().unwrap_or_else(|| name.to_string());
                match kind {
                    "start" => {
                        if !atoms.contains_key(name) {
                            progress::grow(1); // new dependency, not in the plan
                        }
                        let n = progress::take();
                        nums.insert(name.to_string(), n);
                        progress::line(progress::Stage::Installing, n, &atom);
                    }
                    "done" => {
                        if let Some(&n) = nums.get(name) {
                            done_pkgs += 1;
                            progress::line(progress::Stage::Completed, n, &atom);
                        }
                    }
                    _ => {}
                }
            };
            let timer = logbook::Timer::start();
            let res = rootops::sysupgrade(&ignores, &mut show);
            let res = match res {
                // The plan promised upgrades but nothing was installed:
                // never report that as success.
                Ok(()) if !official_upgrades.is_empty() && done_pkgs == 0 => Err(format!(
                    "libalpm queued {} package(s) and installed none, but the plan lists {} \
                     (first sync repo for those names is not the newer one?)",
                    tx_size.unwrap_or(0),
                    official_upgrades.len()
                )),
                other => other,
            };
            match res {
                Ok(()) => {
                    if !official_upgrades.is_empty() {
                        let names: Vec<String> = official_upgrades
                            .iter()
                            .map(|(n, _, _, _)| n.clone())
                            .collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    true
                }
                Err(e) => {
                    progress::status_break();
                    eprintln!("{} {}", ">>> Error:".red().bold(), e);
                    progress::status_resume();
                    false
                }
            }
        };

        // A failed repo upgrade usually needs a human; --keep-going
        // pushes on anyway rather than burying the error under an AUR
        // build against a half-upgraded system.
        let ok2 = if cli.pretend {
            true
        } else if !ok1 && !cli.keep_going {
            eprintln!(
                "{} the official-repo upgrade failed - skipping the AUR upgrade. Pass {} to continue anyway.",
                ">>> Error:".red().bold(),
                "--keep-going".cyan()
            );
            false
        } else {
            // Plan prompt already answered for the whole -u run.
            aur_upgrade_names(
                &aur_plan,
                false,
                cli.skippgp,
                cli.no_sandbox,
                cli.skip_srcinfo_regen,
                cli.unshare_net_build,
            )
        };
        progress::finish();

        println!();
        println!("{} Auto-cleaning packages...", ">>>".green().bold());

        if !cli.pretend {
            if ok1 && ok2 && !runtime::any_failures() {
                clear_resume_state();
            } else if !save_failed_resume(&cli) {
                eprintln!(">>> Warning: not everything upgraded cleanly - state kept for `emerge --resume`.");
            }
        }
        return Ok(());
    }

    // 4. Depclean: orphans via libalpm, never remove world entries.
    if cli.depclean {
        crate::candy::calculating_deps_line();
        println!(">>> Checking for orphaned packages...");

        {
            // Orphans kept only because world lists them are protected.
            let mut world_bare: HashSet<String> = HashSet::new();
            if is_safe_path(WORLD_SET_FILE) {
                if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
                    world_bare = io::BufReader::new(file)
                        .lines()
                        .map_while(Result::ok)
                        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                        .map(|l| l.trim().split('/').last().unwrap_or("").to_string())
                        .collect();
                }
            }
            let protected: Vec<String> = crate::alpm_db::orphan_names()
                .into_iter()
                .filter(|p| world_bare.contains(p))
                .collect();

            // Closure = orphans + deps they free; --exclude'd ones (and what
            // they need) are kept, so the list equals what is removed.
            let mut keep = world_bare.clone();
            let mut all_dropped: Vec<String> = Vec::new();
            let mut orphans;
            loop {
                orphans = crate::alpm_db::orphan_closure(&keep);
                let (_, dropped) = runtime::split_excluded(&orphans);
                if dropped.is_empty() {
                    break;
                }
                keep.extend(dropped.iter().cloned());
                all_dropped.extend(dropped);
            }
            if !protected.is_empty() {
                if cli.show_protected {
                    println!();
                    println!(
                        "{} {} orphan(s) not removed (listed in world, install reason is dependency):",
                        ">>>".yellow().bold(),
                        protected.len()
                    );
                    for p in &protected {
                        println!("    {}", p);
                    }
                    println!();
                } else {
                    println!(
                        ">>> {} package(s) skipped (tracked in world). Pass {} to list them.",
                        protected.len(),
                        "--show-protected".cyan()
                    );
                }
            }

            // --exclude protects from removal as well as from installation.
            runtime::report_excluded(&all_dropped);

            if orphans.is_empty() {
                println!();
                println!(">>> No orphaned packages were found on your system.");
                return Ok(());
            }

            println!();
            for o in &orphans {
                println!("[{}] {}", "unmerge".red().bold(), o);
            }
            println!();
            println!("Total: {} orphaned package(s) to remove", orphans.len());
            println!();

            if cli.pretend || !confirm_action(cli.ask, "unmerge") {
                return Ok(());
            }
            if !cli.ask {
                progress::countdown("Unmerging", 5);
            }
            // The list is already the full closure: remove exactly it,
            // one package at a time so the progress lines are real.
            let (_ok, removed) = unmerge_loop(&orphans);
            logbook::log_unmerge(&removed);
        }
        return Ok(());
    }

    // 5. Unmerge (remove)
    if cli.unmerge {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: Specify packages to remove.");
            std::process::exit(1);
        }
        // Only installed packages can be unmerged; fail before the
        // warning / countdown instead of after it.
        let (installed_targets, not_installed): (Vec<String>, Vec<String>) =
            target_pkgs.iter().cloned().partition(|p| {
                crate::alpm_db::installed_version(p.split('/').last().unwrap_or(p)).is_some()
            });
        if installed_targets.is_empty() {
            eprintln!(
                "{} not installed: {}",
                ">>> Error:".red().bold(),
                not_installed.join(", ")
            );
            std::process::exit(1);
        }
        for m in &not_installed {
            eprintln!(
                "{} not installed, skipped: {}",
                ">>> Warning:".yellow().bold(),
                m
            );
        }
        let target_pkgs = installed_targets;

        let star = " *".yellow().bold();
        println!(
            "{} This action can remove important packages! In order to be safer, use",
            star
        );
        println!(
            "{} `emerge -pc` to check for orphaned packages before",
            star
        );
        println!("{} removing packages.", star);
        println!();
        // Capture repo/name before removal for --undo.
        let mut unmerge_atoms: Vec<String> = Vec::new();
        let repos = get_pkg_repos_batch(&target_pkgs);
        let mut all_selected: Vec<String> = Vec::new();
        for p in &target_pkgs {
            let bare = p.split('/').last().unwrap_or(p);
            let ver = crate::alpm_db::installed_version(bare).unwrap_or_else(|| "?".to_string());
            // "None" = no single sync repo has this version (local/AUR/ABS
            // build): shown bare, saved as Err/ for --undo.
            let (shown, atom) = match repos.get(bare) {
                Some(Some(r)) if r != "None" => {
                    (format!("{}/{}", r, bare), format!("{}/{}", r, bare))
                }
                Some(Some(_)) => (bare.to_string(), format!("Err/{}", bare)),
                _ => (bare.to_string(), bare.to_string()),
            };
            unmerge_atoms.push(atom);
            println!(" {}", shown);
            println!("    selected: {}", ver);
            println!("   protected: none");
            println!("     omitted: none");
            all_selected.push(format!("={}-{}", shown, ver));
        }
        println!();
        println!("All selected packages: {}", all_selected.join(" "));
        println!();
        println!(
            "{} {} packages are slated for removal.",
            ">>>".green().bold(),
            "'Selected'".yellow().bold()
        );
        println!(
            "{} {} and {} packages will not be removed.",
            ">>>".green().bold(),
            "'Protected'".green(),
            "'omitted'".cyan()
        );
        println!();

        // Real emerge -C removes unconditionally (RemoveMode::Unmerge =
        // -Rdd --nosave via libalpm). --pretend only lists the plan.
        if cli.pretend {
            return Ok(());
        }
        if !confirm_action(cli.ask, "unmerge") {
            return Ok(());
        }
        // Portage waits 5s before -C unless --ask already confirmed it.
        if !cli.ask {
            progress::countdown("Unmerging", 5);
        }
        let (_all_ok, removed) = unmerge_loop(&target_pkgs);
        if !removed.is_empty() {
            if let Err(e) = remove_from_world_set(&removed) {
                eprintln!(
                    ">>> Warning: package(s) unmerged but world was not updated: {:#}",
                    e
                );
            }
            // Only what really went away is undoable.
            let done_atoms: Vec<String> = target_pkgs
                .iter()
                .zip(&unmerge_atoms)
                .filter(|(p, _)| removed.contains(*p))
                .map(|(_, a)| a.clone())
                .collect();
            save_last_action(LastAction::Unmerge, &done_atoms);
            logbook::log_unmerge(&removed);
        }
        return Ok(());
    }

    // 6. Install
    if !target_pkgs.is_empty() {
        maybe_news_banner();
        if !cli.pretend {
            save_resume_state(&build_resume_args(&cli, &target_pkgs, has_world));
        }

        // --ask is answered once at the plan prompt (confirm_merge); the
        // install itself never prompts, matching Portage.
        // After the plan prompt (or when --ask was off), never re-prompt
        // pacman/makepkg. Security scanner prompts are separate.
        let ask_pkgs = false;

        let mut success: bool;
        let mut installed_infos: Vec<PkgInfo> = Vec::new();
        // Missing from repos+AUR (report at end; rest still install).
        let mut not_found: Vec<String> = Vec::new();

        // Partition by source so a mixed batch
        // (`aur/foo abs/bar repo/baz`) routes each atom correctly.
        // Global --abs/--aur only claim bare names; an explicit
        // `repo/name` always stays official, and `abs/`/`aur/` always
        // win over the flag.
        let mut abs_pkgs: Vec<String> = Vec::new();
        let mut aur_pkgs: Vec<String> = Vec::new();
        let mut rest_pkgs: Vec<String> = Vec::new();
        for p in &target_pkgs {
            if let Some(name) = p.strip_prefix("abs/") {
                abs_pkgs.push(name.to_string());
            } else if let Some(name) = p.strip_prefix("aur/") {
                aur_pkgs.push(name.to_string());
            } else if cli.abs {
                abs_pkgs.push(p.split('/').last().unwrap_or(p).to_string());
            } else if cli.aur {
                // repo/name is official even under --aur
                if p.contains('/') {
                    rest_pkgs.push(p.clone());
                } else {
                    aur_pkgs.push(p.clone());
                }
            } else {
                rest_pkgs.push(p.clone());
            }
        }

        // Pure ABS batch (no AUR/repo atoms): keep the dedicated path.
        if !abs_pkgs.is_empty() && aur_pkgs.is_empty() && rest_pkgs.is_empty() {
            // abs_install prints its own plan; pass the real --ask so the
            // confirm prompt runs there (AUR/repo paths confirm in main
            // after print_emerge_plan instead).
            success = abs_install(
                &abs_pkgs,
                cli.pretend,
                cli.ask,
                cli.oneshot,
                cli.skippgp,
                cli.edit,
                cli.autopgp,
                cli.no_sandbox,
                cli.skip_srcinfo_regen,
                cli.unshare_net_build,
                cli.pkgbuild_view,
                false, // own plan
            );
            progress::finish();
        } else if !aur_pkgs.is_empty() && abs_pkgs.is_empty() && rest_pkgs.is_empty() {
            // Pure AUR batch.
            let (pkg_infos, missing_aur) = resolve_aur_split(&aur_pkgs);
            not_found = missing_aur;
            if pkg_infos.is_empty() {
                eprintln!(">>> Error: none of the requested package(s) were found in the AUR:");
                for m in &not_found {
                    eprintln!("    {}", m);
                    packages::print_similar_names(m);
                }
                std::process::exit(1);
            }
            print_emerge_plan(&pkg_infos, cli.tree, cli.deep, &aur_pkgs);
            if cli.pretend {
                return Ok(());
            }
            if !confirm_merge(cli.ask) {
                return Ok(());
            }
            print_emerge_emerging(&pkg_infos);
            let found_names: Vec<String> = pkg_infos.iter().map(|p| p.name.clone()).collect();
            scan_aur_pkgbuilds_or_abort(&found_names);
            success = aur_install(
                &found_names,
                false,
                ask_pkgs,
                cli.oneshot,
                cli.skippgp,
                cli.edit,
                cli.no_sandbox,
                cli.skip_srcinfo_regen,
                cli.unshare_net_build,
                cli.pkgbuild_view,
            );
            if success {
                installed_infos = pkg_infos;
            }
            progress::finish();
        } else if !abs_pkgs.is_empty() {
            // Mixed abs + aur/repo: one plan, one confirm, shared Jobs counter.
            let mut target_pkgs = rest_pkgs.clone();
            target_pkgs.extend(aur_pkgs.iter().cloned());

            let abs_infos: Vec<PkgInfo> = abs_pkgs
                .iter()
                .filter_map(|bare| {
                    let version = packages::abs_get_version(bare);
                    let status = packages::pkg_status(bare, &version);
                    Some(PkgInfo {
                        name: bare.clone(),
                        version,
                        repo: "abs".to_string(),
                        status,
                    })
                })
                .collect();

            let (official_infos, missing) = if target_pkgs.is_empty() {
                (Vec::new(), Vec::new())
            } else {
                probe_official_split(&target_pkgs)
            };
            let (aur_infos, missing_aur): (Vec<PkgInfo>, Vec<String>) =
                if target_pkgs.is_empty() || cli.repos {
                    not_found = missing.clone();
                    (Vec::new(), Vec::new())
                } else {
                    let (ai, ma) = resolve_aur_split(&missing);
                    not_found = ma;
                    (ai, Vec::new())
                };
            let _ = missing_aur;

            let mut all_infos = abs_infos.clone();
            all_infos.extend(official_infos.clone());
            all_infos.extend(aur_infos.clone());

            if all_infos.is_empty() {
                eprintln!(">>> Error: none of the requested package(s) could be resolved:");
                for m in &not_found {
                    eprintln!("    {}", m);
                    packages::print_similar_names(m);
                }
                for a in &abs_pkgs {
                    if !abs_infos.iter().any(|p| p.name == *a) {
                        eprintln!("    abs/{}", a);
                    }
                }
                std::process::exit(1);
            }

            // Include abs/ names in `requested` so --tree keeps them at
            // depth 0 (otherwise nano looks like a dep of something else).
            let mut plan_requested = target_pkgs.clone();
            for a in &abs_pkgs {
                plan_requested.push(format!("abs/{}", a));
            }
            print_emerge_plan(&all_infos, cli.tree, cli.deep, &plan_requested);
            if cli.pretend {
                return Ok(());
            }
            if !confirm_merge(cli.ask) {
                return Ok(());
            }
            print_emerge_emerging(&all_infos);

            progress::begin(all_infos.len());
            success = true;

            // Official first (binary, fast), then ABS, then AUR.
            if !official_infos.is_empty() {
                let official_names: Vec<String> = official_infos
                    .iter()
                    .map(|p| {
                        if p.repo.is_empty() || p.repo == "aur" || p.repo == "abs" {
                            p.name.clone()
                        } else {
                            format!("{}/{}", p.repo, p.name)
                        }
                    })
                    .collect();
                let timer = logbook::Timer::start();
                let world_snapshot = world_set::world_installed_snapshot();
                let (ok, landed_names) = repo_install_landed(&official_names, cli.oneshot);
                success = success && ok;
                world_set::reconcile_world_after_install(&world_snapshot);
                if ok {
                    logbook::log_merge_batch("repo", &official_names, timer.elapsed());
                    installed_infos.extend(official_infos);
                } else {
                    let landed: Vec<PkgInfo> = official_infos
                        .into_iter()
                        .filter(|p| {
                            landed_names
                                .iter()
                                .any(|n| n.split('/').last().unwrap_or(n) == p.name)
                        })
                        .collect();
                    if !landed.is_empty() {
                        let names: Vec<String> = landed.iter().map(|p| p.name.clone()).collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    installed_infos.extend(landed);
                    if !cli.keep_going {
                        progress::finish();
                        // fall through to not_found reporting
                    }
                }
            }

            // Shared --jobsa pool: ABS and AUR build concurrently.
            if (success || cli.keep_going) && (!abs_pkgs.is_empty() || !aur_infos.is_empty()) {
                let aur_found: Vec<String> = aur_infos.iter().map(|p| p.name.clone()).collect();
                let src_ok = packages::source_builds_parallel(
                    &abs_pkgs,
                    &aur_found,
                    cli.oneshot,
                    cli.skippgp,
                    cli.edit,
                    cli.autopgp,
                    cli.no_sandbox,
                    cli.skip_srcinfo_regen,
                    cli.unshare_net_build,
                    cli.pkgbuild_view,
                    ask_pkgs,
                );
                success = success && src_ok;
                if src_ok {
                    if !abs_pkgs.is_empty() {
                        installed_infos.extend(abs_infos);
                    }
                    if !aur_found.is_empty() {
                        installed_infos.extend(aur_infos);
                    }
                } else {
                    // Partial success is hard to track across mixed workers;
                    // keep-going still recorded failures via runtime.
                    if !abs_pkgs.is_empty() {
                        installed_infos.extend(abs_infos);
                    }
                    if !aur_found.is_empty() {
                        installed_infos.extend(aur_infos);
                    }
                }
            }

            progress::finish();
        } else if !aur_pkgs.is_empty() {
            // Explicit aur/ plus official rest (no ABS).
            let mut target_pkgs = rest_pkgs;
            for a in &aur_pkgs {
                // Keep them out of probe_official so they go straight to AUR.
                // probe will mark them missing; resolve_aur_split picks them up.
                target_pkgs.push(a.clone());
            }
            // Fall into the official→AUR branch below.
            let (official_infos, missing) = probe_official_split(&target_pkgs);
            // Force any explicitly-prefixed aur/ into the AUR side even
            // if a same-named package exists in official repos.
            let (mut force_aur_found, mut force_aur_miss): (Vec<PkgInfo>, Vec<String>) =
                resolve_aur_split(&aur_pkgs);
            // Official probe may have claimed an aur/ name that also
            // exists in repos; prefer the explicit prefix.
            let official_infos: Vec<PkgInfo> = official_infos
                .into_iter()
                .filter(|p| !aur_pkgs.iter().any(|a| a == &p.name))
                .collect();
            let missing: Vec<String> = missing
                .into_iter()
                .filter(|m| {
                    let bare = m.split('/').last().unwrap_or(m);
                    !aur_pkgs.iter().any(|a| a == bare)
                })
                .collect();
            // Also resolve non-aur remaining misses via AUR.
            let (extra_aur, extra_miss): (Vec<PkgInfo>, Vec<String>) = if cli.repos {
                (Vec::new(), missing)
            } else {
                resolve_aur_split(&missing)
            };
            force_aur_found.extend(extra_aur);
            force_aur_miss.extend(extra_miss);
            not_found = force_aur_miss;

            let mut all_infos = official_infos.clone();
            all_infos.extend(force_aur_found.clone());
            if all_infos.is_empty() {
                eprintln!(
                    ">>> Error: none of the requested package(s) were found in \
                    official repos or the AUR:"
                );
                for m in &not_found {
                    eprintln!("    {}", m);
                    packages::print_similar_names(m);
                }
                std::process::exit(1);
            }
            print_emerge_plan(&all_infos, cli.tree, cli.deep, &target_pkgs);
            if cli.pretend {
                return Ok(());
            }
            if !confirm_merge(cli.ask) {
                return Ok(());
            }
            print_emerge_emerging(&all_infos);
            success = true;
            if !official_infos.is_empty() {
                let official_names: Vec<String> = official_infos
                    .iter()
                    .map(|p| {
                        if p.repo.is_empty() || p.repo == "aur" || p.repo == "abs" {
                            p.name.clone()
                        } else {
                            format!("{}/{}", p.repo, p.name)
                        }
                    })
                    .collect();
                let timer = logbook::Timer::start();
                let world_snapshot = world_set::world_installed_snapshot();
                let (ok, landed_names) = repo_install_landed(&official_names, cli.oneshot);
                success = ok;
                world_set::reconcile_world_after_install(&world_snapshot);
                if ok {
                    logbook::log_merge_batch("repo", &official_names, timer.elapsed());
                    installed_infos.extend(official_infos);
                } else {
                    let landed: Vec<PkgInfo> = official_infos
                        .into_iter()
                        .filter(|p| {
                            landed_names
                                .iter()
                                .any(|n| n.split('/').last().unwrap_or(n) == p.name)
                        })
                        .collect();
                    if !landed.is_empty() {
                        let names: Vec<String> = landed.iter().map(|p| p.name.clone()).collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    installed_infos.extend(landed);
                }
            }
            if !force_aur_found.is_empty() {
                let aur_found: Vec<String> =
                    force_aur_found.iter().map(|p| p.name.clone()).collect();
                scan_aur_pkgbuilds_or_abort(&aur_found);
                let aur_ok = aur_install(
                    &aur_found,
                    false,
                    ask_pkgs,
                    cli.oneshot,
                    cli.skippgp,
                    cli.edit,
                    cli.no_sandbox,
                    cli.skip_srcinfo_regen,
                    cli.unshare_net_build,
                    cli.pkgbuild_view,
                );
                success = success && aur_ok;
                if aur_ok {
                    installed_infos.extend(force_aur_found);
                }
            }
        } else {
            let target_pkgs = rest_pkgs;
            let (official_infos, missing) = probe_official_split(&target_pkgs);

            if missing.is_empty() {
                // Everything found in official repos.
                print_emerge_plan(&official_infos, cli.tree, cli.deep, &target_pkgs);
                if cli.pretend {
                    return Ok(());
                }
                if !confirm_merge(cli.ask) {
                    return Ok(());
                }
                print_emerge_emerging(&official_infos);
                let timer = logbook::Timer::start();
                let world_snapshot = world_set::world_installed_snapshot();
                let (ok, landed_names) = repo_install_landed(&target_pkgs, cli.oneshot);
                success = ok;
                world_set::reconcile_world_after_install(&world_snapshot);
                installed_infos = if success {
                    logbook::log_merge_batch("repo", &target_pkgs, timer.elapsed());
                    official_infos
                } else {
                    // --keep-going retried one by one, so some of these
                    // are on the system now; world below must only
                    // hear about those.
                    let landed: Vec<PkgInfo> = official_infos
                        .into_iter()
                        .filter(|p| {
                            landed_names
                                .iter()
                                .any(|n| n.split('/').last().unwrap_or(n) == p.name)
                        })
                        .collect();
                    if !landed.is_empty() {
                        let names: Vec<String> = landed.iter().map(|p| p.name.clone()).collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    landed
                };
            } else if cli.repos {
                eprintln!(
                    ">>> Warning: --repos is set; the following package(s) were not \
                    found in official repos and will be skipped (AUR was not searched):"
                );
                for m in &missing {
                    eprintln!("    {}", m);
                }

                if official_infos.is_empty() {
                    eprintln!(">>> Error: no requested packages were found in official repos.");
                    std::process::exit(1);
                }

                print_emerge_plan(&official_infos, cli.tree, cli.deep, &target_pkgs);
                if cli.pretend {
                    // Not everything resolved -- exit non-zero for scripts.
                    std::process::exit(1);
                }
                if !confirm_merge(cli.ask) {
                    return Ok(());
                }
                print_emerge_emerging(&official_infos);

                let official_names: Vec<String> = official_infos
                    .iter()
                    .map(|p| {
                        if p.repo.is_empty() || p.repo == "aur" || p.repo == "abs" {
                            p.name.clone()
                        } else {
                            format!("{}/{}", p.repo, p.name)
                        }
                    })
                    .collect();
                let timer = logbook::Timer::start();
                let world_snapshot = world_set::world_installed_snapshot();
                let (off_success, landed_names) = repo_install_landed(&official_names, cli.oneshot);
                world_set::reconcile_world_after_install(&world_snapshot);
                installed_infos = if off_success {
                    logbook::log_merge_batch("repo", &official_names, timer.elapsed());
                    official_infos
                } else {
                    let landed: Vec<PkgInfo> = official_infos
                        .into_iter()
                        .filter(|p| {
                            landed_names
                                .iter()
                                .any(|n| n.split('/').last().unwrap_or(n) == p.name)
                        })
                        .collect();
                    if !landed.is_empty() {
                        let names: Vec<String> = landed.iter().map(|p| p.name.clone()).collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    landed
                };
                success = false; // partial success overall
            } else if official_infos.is_empty() {
                println!(
                    ">>> Not found in official repos. Searching AUR for '{}'...",
                    missing.join(", ")
                );
                let (pkg_infos, missing_aur) = resolve_aur_split(&missing);
                not_found = missing_aur;
                if pkg_infos.is_empty() {
                    eprintln!(">>> Error: none of the requested package(s) were found in official repos or the AUR:");
                    for m in &not_found {
                        eprintln!("    {}", m);
                        packages::print_similar_names(m);
                    }
                    std::process::exit(1);
                }
                print_emerge_plan(&pkg_infos, cli.tree, cli.deep, &target_pkgs);
                if cli.pretend {
                    return Ok(());
                }
                if !confirm_merge(cli.ask) {
                    return Ok(());
                }
                print_emerge_emerging(&pkg_infos);
                let found_names: Vec<String> = pkg_infos.iter().map(|p| p.name.clone()).collect();
                scan_aur_pkgbuilds_or_abort(&found_names);
                success = aur_install(
                    &found_names,
                    false,
                    ask_pkgs,
                    cli.oneshot,
                    cli.skippgp,
                    cli.edit,
                    cli.no_sandbox,
                    cli.skip_srcinfo_regen,
                    cli.unshare_net_build,
                    cli.pkgbuild_view,
                );
                if success {
                    installed_infos = pkg_infos;
                }
            } else {
                // Mixed: official + AUR.
                println!(
                    ">>> Not found in official repos: '{}'. Searching AUR...",
                    missing.join(", ")
                );
                let (aur_infos, missing_aur) = resolve_aur_split(&missing);
                not_found = missing_aur;
                let mut all_infos = official_infos.clone();
                all_infos.extend(aur_infos.clone());
                if all_infos.is_empty() {
                    eprintln!(">>> Error: none of the requested package(s) were found in official repos or the AUR:");
                    for m in &not_found {
                        eprintln!("    {}", m);
                        packages::print_similar_names(m);
                    }
                    std::process::exit(1);
                }
                print_emerge_plan(&all_infos, cli.tree, cli.deep, &target_pkgs);
                if cli.pretend {
                    return Ok(());
                }
                if !confirm_merge(cli.ask) {
                    return Ok(());
                }
                print_emerge_emerging(&all_infos);

                let official_names: Vec<String> = official_infos
                    .iter()
                    .map(|p| {
                        if p.repo.is_empty() || p.repo == "aur" || p.repo == "abs" {
                            p.name.clone()
                        } else {
                            format!("{}/{}", p.repo, p.name)
                        }
                    })
                    .collect();
                let timer = logbook::Timer::start();
                let world_snapshot = world_set::world_installed_snapshot();
                let (ok, landed_names) = repo_install_landed(&official_names, cli.oneshot);
                success = ok;
                world_set::reconcile_world_after_install(&world_snapshot);
                if success {
                    if !official_names.is_empty() {
                        logbook::log_merge_batch("repo", &official_names, timer.elapsed());
                    }
                    installed_infos.extend(official_infos);
                } else {
                    let landed: Vec<PkgInfo> = official_infos
                        .into_iter()
                        .filter(|p| {
                            landed_names
                                .iter()
                                .any(|n| n.split('/').last().unwrap_or(n) == p.name)
                        })
                        .collect();
                    if !landed.is_empty() {
                        let names: Vec<String> = landed.iter().map(|p| p.name.clone()).collect();
                        logbook::log_merge_batch("repo", &names, timer.elapsed());
                    }
                    installed_infos.extend(landed);
                }

                if !aur_infos.is_empty() {
                    let aur_found_names: Vec<String> =
                        aur_infos.iter().map(|p| p.name.clone()).collect();
                    scan_aur_pkgbuilds_or_abort(&aur_found_names);
                    let aur_success = aur_install(
                        &aur_found_names,
                        false,
                        ask_pkgs,
                        cli.oneshot,
                        cli.skippgp,
                        cli.edit,
                        cli.no_sandbox,
                        cli.skip_srcinfo_regen,
                        cli.unshare_net_build,
                        cli.pkgbuild_view,
                    );
                    if aur_success {
                        installed_infos.extend(aur_infos);
                    } else {
                        success = false;
                    }
                }
            }
        }

        progress::finish();

        if !not_found.is_empty() {
            eprintln!();
            eprintln!(">>> Warning: the following package(s) were not found anywhere (official repos or AUR) and were skipped:");
            for m in &not_found {
                eprintln!("    {}", m);
                packages::print_similar_names(m);
                runtime::record_failure(m, "not found in official repos or the AUR");
            }
            success = false;
        }

        if !cli.pretend {
            if !installed_infos.is_empty() {
                refresh_installed_versions(&mut installed_infos);
                print_emerge_completed(&installed_infos);
            }

            if !cli.oneshot {
                if cli.abs {
                    if success {
                        println!("{} Auto-cleaning packages...", ">>>".green().bold());
                        mark_asexplicit(&target_pkgs);
                        if let Err(e) = add_to_world_set(&target_pkgs, Some("abs")) {
                            eprintln!(
                                ">>> Warning: package(s) built but world was not updated: {:#}",
                                e
                            );
                        }
                    }
                } else if !installed_infos.is_empty() {
                    println!("{} Auto-cleaning packages...", ">>>".green().bold());
                    // world only gets explicitly requested names.
                    let target_bare: HashSet<String> = target_pkgs
                        .iter()
                        .map(|p| p.split('/').last().unwrap_or(p).to_string())
                        .collect();
                    let explicit_infos: Vec<&PkgInfo> = installed_infos
                        .iter()
                        .filter(|p| target_bare.contains(&p.name))
                        .collect();
                    let abs_names: Vec<String> = explicit_infos
                        .iter()
                        .filter(|p| p.repo == "abs")
                        .map(|p| p.name.clone())
                        .collect();
                    let aur_names: Vec<String> = explicit_infos
                        .iter()
                        .filter(|p| p.repo == "aur")
                        .map(|p| p.name.clone())
                        .collect();
                    let official_names: Vec<String> = explicit_infos
                        .iter()
                        .filter(|p| p.repo != "aur" && p.repo != "abs")
                        .map(|p| p.name.clone())
                        .collect();
                    // One world write for official + aur + abs (single message).
                    if let Err(e) = world_set::add_to_world_groups(&[
                        (&official_names, None),
                        (&aur_names, Some("aur")),
                        (&abs_names, Some("abs")),
                    ]) {
                        eprintln!(
                            ">>> Warning: package(s) installed but world was not updated: {:#}",
                            e
                        );
                    }
                    if !aur_names.is_empty() {
                        mark_asexplicit(&aur_names);
                    }
                    if !abs_names.is_empty() {
                        mark_asexplicit(&abs_names);
                    }

                    // Transitive deps → asdeps so depclean can see them.
                    let dep_only_names: Vec<String> = installed_infos
                        .iter()
                        .filter(|p| !target_bare.contains(&p.name))
                        .map(|p| p.name.clone())
                        .collect();
                    if !dep_only_names.is_empty() {
                        mark_asdeps(&dep_only_names);
                    }
                }
            }

            if !success {
                eprintln!(
                    "{} not all requested packages were installed successfully.",
                    ">>> Warning:".yellow().bold()
                );
                save_failed_resume(&cli);
                if runtime::any_failures() {
                    // main() prints the summary and exits non-zero.
                    return Ok(());
                }
                std::process::exit(1);
            } else {
                clear_resume_state();
                let new_names: Vec<String> = installed_infos
                    .iter()
                    .filter(|p| p.status == "N")
                    .map(|p| p.name.clone())
                    .collect();
                if !new_names.is_empty() && !cli.oneshot {
                    save_last_action(LastAction::Install, &new_names);
                }
            }
        }
    }

    // After named pkgs: provision remaining world misses.
    if provision_after_install && !cli.pretend {
        println!();
        if !provision_from_world_set(
            cli.pretend,
            cli.ask,
            cli.verbose,
            cli.err_install,
            cli.no_sandbox,
            cli.skip_srcinfo_regen,
            cli.unshare_net_build,
        )? {
            eprintln!(">>> Warning: not everything from world installed successfully.");
            std::process::exit(1);
        }
    }

    Ok(())
}

#[cfg(test)]
mod action_priority_tests {
    use super::*;

    #[test]
    fn long_flags_keep_argv_order() {
        let argv = vec!["emerge".into(), "--depclean".into(), "--update".into()];
        let active = vec![ActionKind::Update, ActionKind::Depclean];
        assert_eq!(
            actions_in_argv_order(&argv, &active),
            vec![ActionKind::Depclean, ActionKind::Update]
        );
    }

    #[test]
    fn short_cluster_left_to_right() {
        let active = vec![ActionKind::Update, ActionKind::Depclean];
        assert_eq!(
            actions_in_argv_order(&["emerge".into(), "-uc".into()], &active),
            vec![ActionKind::Update, ActionKind::Depclean]
        );
        assert_eq!(
            actions_in_argv_order(&["emerge".into(), "-cu".into()], &active),
            vec![ActionKind::Depclean, ActionKind::Update]
        );
    }

    #[test]
    fn scan_not_listed_beside_install_pkgbuild() {
        let cli = Cli::parse_from(["emerge", "--install-pkgbuild", "/tmp/pkg", "--scan"]);
        let active = active_actions(&cli);
        assert!(active.contains(&ActionKind::InstallPkgbuild));
        assert!(!active.contains(&ActionKind::Scan));
    }

    #[test]
    fn order_follows_argv_not_field_order() {
        let active = vec![ActionKind::Search, ActionKind::Depclean, ActionKind::Update];
        let argv: Vec<String> = ["emerge", "-c", "--update", "-s"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            actions_in_argv_order(&argv, &active),
            vec![ActionKind::Depclean, ActionKind::Update, ActionKind::Search]
        );
    }

    #[test]
    fn multiple_actions_text_matches_portage() {
        assert_eq!(
            multiple_actions_message(ActionKind::Search, ActionKind::Depclean),
            "\n!!! Multiple actions requested... Please choose one only.\n!!! 'search' or 'depclean'\n\n"
        );
        assert_eq!(ActionKind::News.portage_name(), "check-news");
    }
}
