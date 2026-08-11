//! Bubblewrap (bwrap) sandbox for the untrusted parts of a build:
//! `pkgver()` / `prepare()` / `build()` / `check()` / `package()` as
//! defined in an AUR/ABS `PKGBUILD`.
//!
//! The static scanner in security.rs/bash_ast.rs is a blocklist: it
//! catches known-bad patterns *before* anything runs, but a PKGBUILD it
//! doesn't flag still gets to execute arbitrary shell as the build user.
//! This module is the second, independent layer for that case: whatever
//! the scanner missed only ever runs inside a namespace that can't see
//! the real $HOME (no ssh keys, no gpg secret keyring, no browser
//! profile, no other projects on disk) and can't write anywhere outside
//! its own throwaway build directory.
//!
//! Deliberately NOT covered here: dependency installation and the final
//! `pacman -U`. Both need real root and a real view of the live system,
//! and neither one runs a single line of PKGBUILD-authored code (it's
//! just pacman resolving/installing package names) -- so both stay
//! outside the jail and run the normal, trusted way. Only the shell
//! functions the PKGBUILD itself defines run inside bwrap. See
//! `packages::build_with_sandbox` for how the three (or, with
//! `--unshare-net-build`, four) steps are wired together.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const BWRAP_BIN: &str = "/usr/bin/bwrap";

/// Scratch $HOME handed to the sandboxed build. Not the real one -- a
/// malicious `prepare()`/`build()` should find nothing here worth
/// stealing or tampering with.
const SANDBOX_HOME: &str = "/tmp/aura-emerge-sandbox-home";

pub(crate) fn bwrap_available() -> bool {
    Path::new(BWRAP_BIN).exists()
}

/// Builds the `bwrap ... -- makepkg ...` command that runs the
/// build-time PKGBUILD functions in an isolated user/mount/pid/ipc/uts
/// namespace.
///
/// `build_dir` is the *only* path inside the jail that's writable --
/// it's where the AUR/ABS checkout already lives, so makepkg's own
/// $srcdir/$pkgdir (both subdirectories of it by default) are writable
/// for free without needing their own bind rules.
///
/// `extra_dest_dirs` covers the case where the user's own makepkg.conf
/// (`PKGDEST`/`SRCDEST`/`SRCPKGDEST`/`BUILDDIR`) points *outside*
/// `build_dir`, plus aura-emerge's own default persistent `SRCDEST`
/// source cache when the user hasn't set one -- see
/// `packages::resolve_dest_dirs`/`packages::source_cache_dir`, which
/// compute this list against the real, unsandboxed makepkg.conf
/// resolution (system + user config, real `$HOME`). Each `(var, path)` pair gets
/// its own writable bind at that same absolute path inside the jail, so
/// the write the user actually configured still lands where they
/// expect, plus a matching `--setenv` so the sandboxed makepkg -- whose
/// own config resolution runs against the fake `$HOME` below, which
/// hides any *user-level* makepkg.conf that set it in the first place --
/// picks up the same value rather than silently falling back to
/// makepkg's own default (`build_dir`).
///
/// Known gap: if `/etc/makepkg.conf` itself (visible inside the jail,
/// unlike user-level config) *also* sets one of these to something
/// different from what was resolved outside, its `source`d assignment
/// runs after this function's `--setenv` and wins. Rare in practice --
/// Arch ships these commented out by default in `/etc/makepkg.conf`,
/// and a machine actively relying on *both* a system-wide and a
/// diverging user-level override for the same variable would be an
/// unusual setup on its own.
///
/// Network stays shared (`--share-net`) by default since source downloads
/// still need it; everything else (`--unshare-all`: user, ipc, pid, net,
/// uts, cgroup -- net is then optionally re-added by --share-net) is
/// torn down. Pass `net = false` to leave it torn down instead -- see
/// `net`'s own doc below for what that actually isolates and what it
/// doesn't.
///
/// `caller_args` should NOT include `-s`/`--syncdeps` or `-i`/`--install`
/// -- see the module doc for why those two stay outside the sandbox.
///
/// `net`: whether this specific invocation gets `--share-net`. Callers
/// building with `--unshare-net-build` (see `packages::build_with_sandbox`)
/// pass `true` for the download/extract/`prepare()` phase (`makepkg
/// --nobuild`) -- those are the declared, checksum/PGP-verified
/// `source=()` entries, the legitimate/audited use of the network here --
/// and `false` for the actual `build()`/`check()`/`package()` phase
/// (`makepkg --noextract`), so a build step that reaches for the network
/// *outside* the declared source list (an unvendored `cargo build`
/// resolving crates.io mid-compile is exactly the case that prompted
/// this) fails loudly instead of quietly succeeding with unaudited
/// traffic from the untrusted part of the build.
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
    // Whole real filesystem, read-only: build() still needs to see
    // /usr, /etc/makepkg.conf, toolchains, etc. -- it just can't touch
    // any of it.
    cmd.args(["--ro-bind", "/", "/"]);
    // Everything below MUST come AFTER the ro-bind above, not before:
    // bwrap applies bind rules in argument order, so a rule issued
    // earlier for a path under "/" gets silently clobbered by the later
    // "--ro-bind / /" once that runs. That bit us twice already:
    //   - "--proc /proc" / "--dev /dev" issued before the ro-bind meant
    //     the sandbox's /proc and /dev were actually the host's real
    //     ones, read-only -- hence "/dev/null: Permission denied".
    //   - "--tmpfs /tmp" issued before the ro-bind meant /tmp was
    //     read-only too, breaking the mkdir for the fake $HOME mount
    //     point under it, further down.
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--tmpfs", "/tmp"]);
    // The one writable exception: the build's own directory.
    cmd.args(["--bind", &build_dir_s, &build_dir_s]);
    // Any configured PKGDEST/SRCDEST/SRCPKGDEST/BUILDDIR that lives
    // outside build_dir gets its own writable bind + matching env var,
    // best-effort-created first since a fresh destination directory

    // (e.g. a PKGDEST nobody's built into yet) may not exist yet.
    for (var, path) in extra_dest_dirs {
        let _ = std::fs::create_dir_all(path);
        let path_s = path.to_string_lossy().to_string();
        cmd.args(["--bind", &path_s, &path_s]);
        cmd.args(["--setenv", *var, &path_s]);
    }
    // Isolated, empty $HOME -- overrides whatever the "/" ro-bind above
    // would otherwise expose at this path.
    cmd.args(["--tmpfs", &fake_home_s]);
    cmd.args(["--setenv", "HOME", &fake_home_s]);
    cmd.args(["--unsetenv", "XDG_CONFIG_HOME"]);
    cmd.args(["--unsetenv", "XDG_CACHE_HOME"]);

    // Read-only PGP public keyring so source-signature verification
    // (already primed by ensure_pgp_keys() outside the sandbox, before
    // this runs) keeps working. Read-only means a compromised build
    // script can see imported public keys but can't plant a trusted key
    // of its own or touch anything under the real ~/.gnupg.
    if let Some(gnupg) = real_gnupg_home {
        if gnupg.exists() {
            let dest = fake_home.join(".gnupg");
            let dest_s = dest.to_string_lossy().to_string();
            let src_s = gnupg.to_string_lossy().to_string();
            cmd.args(["--ro-bind", &src_s, &dest_s]);
        }
    }

    cmd.args(["--chdir", &build_dir_s]);
    cmd.arg("--");
    cmd.arg(makepkg_bin);
    cmd.args(caller_args);
    cmd
}