//! Bubblewrap (bwrap) sandbox for the untrusted PKGBUILD functions:
//! `pkgver()`/`prepare()`/`build()`/`check()`/`package()`.
//!
//! The static scanner (security.rs/bash_ast.rs) is a blocklist -- a
//! PKGBUILD it misses still runs arbitrary shell as the build user.
//! This is the second layer: whatever slips past runs in a namespace
//! that can't see the real $HOME (no ssh/gpg/browser secrets, no other
//! projects) and can't write outside its own throwaway build dir.
//!
//! NOT covered: dependency install and the final `pacman -U` -- both
//! need real root, neither runs PKGBUILD code, so both stay outside
//! the jail. See `packages::build_with_sandbox` for how it's wired up.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const BWRAP_BIN: &str = "/usr/bin/bwrap";

/// Scratch $HOME for the sandboxed build -- not the real one, so a
/// malicious `prepare()`/`build()` finds nothing worth stealing.
const SANDBOX_HOME: &str = "/tmp/aura-emerge-sandbox-home";

pub(crate) fn bwrap_available() -> bool {
    Path::new(BWRAP_BIN).exists()
}

/// Fixes "rustup could not choose a version of cargo to run" in a
/// sandboxed `build()`: rustup's `$RUSTUP_HOME` (default
/// `$HOME/.rustup`) is now empty under the fake `$HOME`, so it can't
/// find its settings, even though the shims/toolchain are still
/// reachable via `--ro-bind /`. Fix: `--setenv RUSTUP_HOME` back to its
/// real path.
///
/// `$CARGO_HOME` is NOT restored the same way -- `~/.cargo` can hold a
/// real secret (`credentials.toml`), so it gets a fresh writable dir
/// inside `build_dir` instead; cargo just re-fetches deps into it.
fn rustup_env(build_dir: &Path) -> Vec<(String, String)> {
    let mut env = Vec::new();

    let real_rustup_home = std::env::var("RUSTUP_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".rustup")));
    if let Some(rustup_home) = real_rustup_home {
        if rustup_home.is_dir() {
            env.push(("RUSTUP_HOME".to_string(), rustup_home.to_string_lossy().to_string()));
        }
    }

    let cargo_home = build_dir.join(".aura-emerge-sandbox-cargo-home");
    if std::fs::create_dir_all(&cargo_home).is_ok() {
        env.push(("CARGO_HOME".to_string(), cargo_home.to_string_lossy().to_string()));
    }

    env
}

/// Builds the `bwrap ... -- makepkg ...` command running the build-time
/// PKGBUILD functions in an isolated namespace.
///
/// `build_dir` is the only writable path -- it's the AUR/ABS checkout,
/// so makepkg's own $srcdir/$pkgdir are writable for free.
///
/// `extra_dest_dirs`: any `PKGDEST`/`SRCDEST`/`SRCPKGDEST`/`BUILDDIR`
/// resolved outside `build_dir` (see `packages::resolve_dest_dirs`),
/// each gets its own writable bind + matching `--setenv`, since the
/// sandboxed makepkg can't see the user-level config that set it.
/// Known gap: a differing `/etc/makepkg.conf` value (visible in the
/// jail) still wins over our `--setenv` -- rare in practice.
///
/// `net`: whether this call gets `--share-net`. Callers using
/// `--unshare-net-build` pass `true` for the `prepare()`/download phase
/// (verified `source=()` entries -- the legitimate use of network
/// here), `false` for `build()`/`check()`/`package()`, so anything
/// reaching for the network outside the declared sources (e.g. `cargo
/// build` hitting crates.io mid-compile) fails loudly instead of
/// succeeding quietly.
///
/// `caller_args` must NOT include `-s`/`-i` -- see module doc.
pub(crate) fn sandboxed_makepkg(
    makepkg_bin: &str,
    build_dir: &Path,
    caller_args: &[&str],
    real_gnupg_home: Option<&Path>,
    extra_dest_dirs: &[(&str, PathBuf)],
    net: bool,
) -> Command {
    let build_dir_s = build_dir.to_string_lossy().to_string();
    let fake_home = PathBuf::from(SANDBOX_HOME);
    let fake_home_s = fake_home.to_string_lossy().to_string();

    let mut cmd = Command::new(BWRAP_BIN);
    cmd.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
    ]);
    if net {
        cmd.arg("--share-net");
    }
    // Whole real fs, read-only: build() needs to see /usr, makepkg.conf,
    // toolchains, etc., just can't touch any of it.
    cmd.args(["--ro-bind", "/", "/"]);
    // Everything below MUST stay after this ro-bind (bwrap applies bind
    // rules in order; an earlier rule under "/" gets clobbered by it).
    // Bit us before: --proc/--dev too early -> host's real read-only
    // /proc,/dev ("Permission denied"); --tmpfs /tmp too early -> /tmp
    // read-only, breaking the fake-$HOME mkdir below.
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--tmpfs", "/tmp"]);
    // The one writable exception: the build's own directory.
    cmd.args(["--bind", &build_dir_s, &build_dir_s]);
    // Any configured PKGDEST/SRCDEST/SRCPKGDEST/BUILDDIR outside
    // build_dir gets its own writable bind + env var (best-effort
    // created first, since it may not exist yet).
    for (var, path) in extra_dest_dirs {
        let _ = std::fs::create_dir_all(path);
        let path_s = path.to_string_lossy().to_string();
        cmd.args(["--bind", &path_s, &path_s]);
        cmd.args(["--setenv", *var, &path_s]);
    }
    // Isolated, empty $HOME -- overrides what the "/" ro-bind exposes here.
    cmd.args(["--tmpfs", &fake_home_s]);
    cmd.args(["--setenv", "HOME", &fake_home_s]);
    cmd.args(["--unsetenv", "XDG_CONFIG_HOME"]);
    cmd.args(["--unsetenv", "XDG_CACHE_HOME"]);

    // Read-only PGP keyring (imported by ensure_pgp_keys() outside the
    // sandbox) so signature verification still works, but a compromised
    // build can't plant its own key or touch the real ~/.gnupg.
    if let Some(gnupg) = real_gnupg_home {
        if gnupg.exists() {
            let dest = fake_home.join(".gnupg");
            let dest_s = dest.to_string_lossy().to_string();
            let src_s = gnupg.to_string_lossy().to_string();
            cmd.args(["--ro-bind", &src_s, &dest_s]);
        }
    }

    // rustup: see rustup_env's doc comment.
    for (k, v) in rustup_env(build_dir) {
        cmd.args(["--setenv", &k, &v]);
    }

    cmd.args(["--chdir", &build_dir_s]);
    cmd.arg("--");
    cmd.arg(makepkg_bin);
    cmd.args(caller_args);
    cmd
}