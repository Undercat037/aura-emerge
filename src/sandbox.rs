//! bwrap sandbox for untrusted PKGBUILD phases (`pkgver`/`prepare`/
//! `build`/`check`/`package`). Second layer after the static scanner:
//! no real $HOME, no /run (session bus / agents), cleared env, writes

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

pub(crate) const BWRAP_BIN: &str = "/usr/bin/bwrap";

/// Scratch $HOME for the sandboxed build -- not the real one, so a
/// malicious `prepare()`/`build()` finds nothing worth stealing.
const SANDBOX_HOME: &str = "/tmp/aura-emerge-sandbox-home";

/// Scratch for fakeroot shim / generated makepkg.conf / public keyring.
/// Must remain visible inside the sandbox (`--tmpfs /run` and `/tmp`
/// hide those trees). Prefer `/var/tmp`; never `build_dir`.
const FAKEROOT_SHIM_ROOT: &str = "/var/tmp";

fn is_masked_inside_sandbox(p: &Path) -> bool {
    let s = p.to_string_lossy();
    for prefix in ["/run", "/tmp", "/home", "/mnt", "/media"] {
        if s == prefix || s.starts_with(&(prefix.to_string() + "/")) {
            return true;
        }
    }
    false
}

/// Scratch parent: XDG_RUNTIME_DIR only if not under a sandbox tmpfs mask.
fn shim_root() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(&xdg);
        if is_real_dir(&p) && !is_masked_inside_sandbox(&p) {
            return p;
        }
    }
    PathBuf::from(FAKEROOT_SHIM_ROOT)
}

/// Replaced by empty tmpfs after `--ro-bind / /` (before writable binds).
/// /home secrets, /run session bus+agents, /mnt+/media other disks.
const HIDDEN_DIRS: &[&str] = &["/home", "/run", "/mnt", "/media"];

/// With network on, DNS still has to resolve: `/etc/resolv.conf` is
/// usually a symlink into one of these (now hidden by `--tmpfs /run`).
const NET_RO_BINDS: &[&str] = &["/run/systemd/resolve", "/run/NetworkManager"];

/// The only environment variables the sandboxed makepkg inherits
/// (plus `LC_*`, `HOME`, `PATH` and our own `--setenv`s). Everything
/// else -- SSH_AUTH_SOCK, DBUS_SESSION_BUS_ADDRESS, *_TOKEN, ... -- is

const ENV_PASSTHROUGH: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "TZ",
    "USER",
    "LOGNAME",
    "SHELL",
    "PACKAGER",
    "SOURCE_DATE_EPOCH",
];

/// Only passed when the call has network (`net: true`).
const ENV_PROXY: &[&str] = &[
    "http_proxy",
    "https_proxy",
    "ftp_proxy",
    "all_proxy",
    "no_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "FTP_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
];

/// A real directory (not a symlink to one, not missing) -- the only
/// kind `--tmpfs` can safely mount over under a read-only root.
fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Config files whose *target* lives in a directory we hide.
///
/// `/etc/makepkg.conf` (and `/etc/makepkg.conf.d/*`) are often

fn exposed_targets(candidates: &[PathBuf], hidden: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        let Ok(real) = std::fs::canonicalize(c) else {
            continue;
        };
        if real == *c || out.contains(&real) {
            continue;
        }
        if hidden.iter().any(|h| real.starts_with(h)) {
            out.push(real);
        }
    }
    out
}

fn exposed_config_targets(real_home: Option<&Path>) -> Vec<PathBuf> {
    let mut hidden: Vec<PathBuf> = HIDDEN_DIRS.iter().map(PathBuf::from).collect();
    if let Some(h) = real_home {
        hidden.push(h.to_path_buf());
    }
    let mut candidates = vec![
        PathBuf::from("/etc/makepkg.conf"),
        PathBuf::from("/etc/makepkg.conf.d"),
    ];
    if let Ok(rd) = std::fs::read_dir("/etc/makepkg.conf.d") {
        candidates.extend(rd.flatten().map(|e| e.path()));
    }
    exposed_targets(&candidates, &hidden)
}

pub(crate) fn bwrap_available() -> bool {
    Path::new(BWRAP_BIN).exists()
}

/// Per-build scratch dir, for files makepkg should read but the
/// untrusted `prepare()`/`build()` shouldn't be able to rewrite (the
/// fakeroot shim, the generated makepkg.conf carrying emerge.conf's

pub(crate) fn scratch_dir(build_dir: &Path) -> PathBuf {
    fakeroot_shim_scratch_dir(build_dir)
}

struct RustEnv {
    env: Vec<(String, String)>,
    /// Real paths re-exposed read-only on top of the hidden /home.
    ro_binds: Vec<PathBuf>,
}

/// Fixes "rustup could not choose a version of cargo to run" in a
/// sandboxed `build()`: rustup's `$RUSTUP_HOME` (default
/// `$HOME/.rustup`) is empty under the fake `$HOME` (and, now that

fn rustup_env(build_dir: &Path) -> RustEnv {
    let mut out = RustEnv {
        env: Vec::new(),
        ro_binds: Vec::new(),
    };

    let real_home = std::env::var_os("HOME").map(PathBuf::from);
    let real_rustup_home = std::env::var("RUSTUP_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| real_home.as_ref().map(|h| h.join(".rustup")));
    if let Some(rustup_home) = real_rustup_home {
        if rustup_home.is_dir() {
            out.env.push((
                "RUSTUP_HOME".to_string(),
                rustup_home.to_string_lossy().to_string(),
            ));
            out.ro_binds.push(rustup_home);
        }
    }
    if let Some(h) = &real_home {
        let cargo_bin = h.join(".cargo").join("bin");
        if cargo_bin.is_dir() {
            out.ro_binds.push(cargo_bin);
        }
    }

    let cargo_home = build_dir.join(".aura-emerge-sandbox-cargo-home");
    if std::fs::create_dir_all(&cargo_home).is_ok() {
        out.env.push((
            "CARGO_HOME".to_string(),
            cargo_home.to_string_lossy().to_string(),
        ));
    }

    out
}

// ── per-build scratch dir ─────────────────────────────────────────────────────

/// build_dir -> its scratch dir, so the fetch/build/package
/// `sandboxed_makepkg` calls of one build reuse one dir.
static SCRATCH_DIRS: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();

fn scratch_dirs() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    SCRATCH_DIRS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 96 random bits as hex. If /dev/urandom is unreadable the fallback
/// is guessable -- fine, because safety doesn't rest on the name being
/// secret but on the *exclusive* `mkdir` below.
fn random_token() -> String {
    let mut buf = [0u8; 12];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut h);
        std::process::id().hash(&mut h);
        buf[..8].copy_from_slice(&h.finish().to_le_bytes());
    }
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Once per build_dir: exclusive mkdir 0700 under shim_root() (random name).
fn fakeroot_shim_scratch_dir(build_dir: &Path) -> PathBuf {
    let mut map = scratch_dirs().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = map.get(build_dir) {
        return dir.clone();
    }
    let root = shim_root();
    let mut candidate = root.join(".aura-emerge-sandbox-unavailable");
    for _ in 0..8 {
        candidate = root.join(format!(".aura-emerge-sandbox-{}", random_token()));
        if std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&candidate)
            .is_ok()
        {
            map.insert(build_dir.to_path_buf(), candidate.clone());
            return candidate;
        }
    }
    // Could not create one (unwritable root?): hand back the last
    // candidate; every later write into it fails and callers degrade
    // to "no shim"/"no keyring" instead of writing somewhere unsafe.
    candidate
}

/// RAII cleanup for the scratch dir -- it lives outside `build_dir`,
/// so something has to delete it explicitly. Construct one at the top
/// of `build_with_sandbox` so every exit path cleans up.
pub(crate) struct FakerootShimGuard {
    build_dir: PathBuf,
}

impl FakerootShimGuard {
    pub(crate) fn new(build_dir: &Path) -> Self {
        let _ = fakeroot_shim_scratch_dir(build_dir);
        Self {
            build_dir: build_dir.to_path_buf(),
        }
    }
}

impl Drop for FakerootShimGuard {
    fn drop(&mut self) {
        let dir = scratch_dirs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.build_dir);
        if let Some(dir) = dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

// ── fakeroot shim ─────────────────────────────────────────────────────────────

/// Fixes "cp: cannot preserve ownership: Invalid argument" in `package()`
/// (confirmed with strace): `package()` runs under `fakeroot`, which looks

fn fakeroot_shim_dir(build_dir: &Path, extra_dest_dirs: &[(&str, PathBuf)]) -> Option<PathBuf> {
    // Pinned: a `fakeroot` found via the caller's $PATH could be a
    // user-writable file. Only fall back to PATH if the system one is
    // missing.
    let system_fakeroot = PathBuf::from("/usr/bin/fakeroot");
    let real_fakeroot = if system_fakeroot.is_file() {
        system_fakeroot
    } else {
        std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join("fakeroot"))
            .find(|p| p.is_file())?
    };

    let shim_dir = fakeroot_shim_scratch_dir(build_dir);

    // Fresh nested mount namespace, so build_dir/extra_dest_dirs need
    // re-binding writable or package() just hits read-only. /dev needs
    // its own --dev-bind (not folded into "/"): plain --bind is nodev,

    let mut inner_binds = format!(
        "--ro-bind / / --dev-bind /dev /dev --bind {0} {0}",
        shq(build_dir)
    );
    for (_, path) in extra_dest_dirs {
        inner_binds.push_str(&format!(" --bind {0} {0}", shq(path)));
    }

    write_shim(&shim_dir, &real_fakeroot, &inner_binds)?;
    Some(shim_dir)
}

/// Writes `<shim_dir>/fakeroot`. `shim_dir` must be our own 0700 dir
/// (see `fakeroot_shim_scratch_dir`); the file is still created with
/// `O_EXCL` (never follows a symlink) and moved into place with

fn write_shim(shim_dir: &Path, real_fakeroot: &Path, inner_binds: &str) -> Option<()> {
    // --die-with-parent: this nested bwrap doesn't outlive the outer
    // makepkg if it's killed. --new-session: matches the outer
    // sandbox's own flag, cutting off TIOCSTI and other terminal-based

    let real = shq(real_fakeroot);
    let script = format!(
        "#!/bin/sh\n\
         case \"$1\" in -v|--version|-h|--help) exec {real} \"$@\" ;; esac\n\
         [ -n \"$FAKEROOTKEY\" ] && exec {real} \"$@\"\n\
         exec {bwrap} --unshare-user --die-with-parent --new-session --cap-drop ALL --uid 0 --gid 0 {binds} -- {real} \"$@\"\n",
        real = real,
        bwrap = BWRAP_BIN,
        binds = inner_binds,
    );

    let tmp = shim_dir.join(".fakeroot.new");
    let _ = std::fs::remove_file(&tmp); // unlink never follows a symlink
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(&tmp)
        .ok()?;
    f.write_all(script.as_bytes()).ok()?;
    f.set_permissions(std::fs::Permissions::from_mode(0o755))
        .ok()?;
    drop(f);
    std::fs::rename(&tmp, shim_dir.join("fakeroot")).ok()
}

/// Quotes a path for the shim's shell script (our own paths, not
/// attacker input, but cheap to be safe anyway).
fn shq(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

// ── keyring ───────────────────────────────────────────────────────────────────

/// A copy of the PUBLIC half of the real GnuPG home, for signature
/// verification inside the sandbox.

fn sandbox_gnupg_home(build_dir: &Path) -> Option<PathBuf> {
    let dst = fakeroot_shim_scratch_dir(build_dir).join("gnupg");
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::DirBuilder::new().mode(0o700).create(&dst).ok()?;
    let _ = std::fs::write(dst.join("gpg.conf"), "batch\nno-tty\nkeyid-format long\n");
    Some(dst)
}

/// Import public keys into a sandbox GNUPGHOME via keyserver.
pub(crate) fn sandbox_recv_keys(gnupg: &Path, keys: &[String], keyserver: &str) -> usize {
    if keys.is_empty() || !gnupg.is_dir() {
        return 0;
    }
    let mut ok = 0usize;
    for key in keys {
        let status = std::process::Command::new("gpg")
            .args([
                "--homedir",
                gnupg.to_str().unwrap_or(""),
                "--batch",
                "--yes",
                "--keyserver",
                keyserver,
                "--recv-keys",
                key,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if status {
            ok += 1;
        }
    }
    ok
}

/// Best-effort: dump host public keys into the sandbox ring.
fn seed_from_host_export(real: &Path, dst: &Path) {
    if !real.is_dir() {
        return;
    }
    let export = std::process::Command::new("gpg")
        .args([
            "--homedir",
            real.to_str().unwrap_or(""),
            "--batch",
            "--export",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = export else {
        return;
    };
    if out.stdout.is_empty() {
        return;
    }
    use std::io::Write;
    let mut child = match std::process::Command::new("gpg")
        .args([
            "--homedir",
            dst.to_str().unwrap_or(""),
            "--batch",
            "--yes",
            "--import",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&out.stdout);
    }
    let _ = child.wait();
}

// ── the sandbox itself ────────────────────────────────────────────────────────

/// Builds the `bwrap ... -- makepkg ...` command running the build-time
/// PKGBUILD functions in an isolated namespace.

pub(crate) fn sandboxed_makepkg(
    makepkg_bin: &str,
    build_dir: &Path,
    caller_args: &[&str],
    real_gnupg_home: Option<&Path>,
    extra_dest_dirs: &[(&str, PathBuf)],
    extra_pgp_keys: &[String],
    net: bool,
) -> Command {
    let build_dir_s = build_dir.to_string_lossy().to_string();
    let fake_home = PathBuf::from(SANDBOX_HOME);
    let fake_home_s = fake_home.to_string_lossy().to_string();
    let real_home = std::env::var_os("HOME").map(PathBuf::from);

    let mut cmd = Command::new(BWRAP_BIN);
    cmd.args(["--die-with-parent", "--new-session", "--unshare-all"]);
    if net {
        cmd.arg("--share-net");
    }
    // Untrusted PKGBUILD code has no business holding any capability, so
    // strip them all; `net: false` gets CAP_NET_ADMIN back just for
    // bringing up `lo` below.
    cmd.args(["--cap-drop", "ALL"]);
    if !net {
        cmd.args(["--cap-add", "CAP_NET_ADMIN"]);
    }
    // Start from an empty environment; the allowlist is set further
    // down. Must come before every --setenv (bwrap applies in order).
    cmd.arg("--clearenv");
    // Whole real fs, read-only: build() needs to see /usr, makepkg.conf,
    // toolchains, etc., just can't touch any of it.
    cmd.args(["--ro-bind", "/", "/"]);
    // Everything below MUST stay after this ro-bind (bwrap applies bind
    // rules in order; an earlier rule under "/" gets clobbered by it).
    // Bit us before: --proc/--dev too early -> host's real read-only

    cmd.args(["--proc", "/proc"]);
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--tmpfs", "/tmp"]);

    // Hide the real home, session runtime dir and other mounts. This
    // is what makes the "can't see the real $HOME" promise true --
    // `--ro-bind / /` alone leaves /home/<user>/.ssh readable and only

    for dir in HIDDEN_DIRS {
        if is_real_dir(Path::new(dir)) {
            cmd.args(["--tmpfs", dir]);
        }
    }
    if let Some(h) = &real_home {
        // $HOME outside /home (/var/home, /data/me, ...).
        if h.is_absolute() && h != Path::new("/") && !h.starts_with("/home") && is_real_dir(h) {
            let s = h.to_string_lossy().to_string();
            cmd.args(["--tmpfs", &s]);
        }
    }
    if net {
        for p in NET_RO_BINDS {
            cmd.args(["--ro-bind-try", p, p]);
        }
    }

    // makepkg.conf symlinked into a hidden dir: bring just that file
    // back, read-only (see exposed_targets).
    for p in exposed_config_targets(real_home.as_deref()) {
        let s = p.to_string_lossy().to_string();
        cmd.args(["--ro-bind", &s, &s]);
    }

    // The one writable exception: the build's own directory.
    cmd.args(["--bind", &build_dir_s, &build_dir_s]);
    // ...with its .install scripts frozen (see doc comment).
    if let Ok(rd) = std::fs::read_dir(build_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let is_install = p.extension().and_then(|e| e.to_str()) == Some("install");
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if is_install && is_file {
                let s = p.to_string_lossy().to_string();
                cmd.args(["--ro-bind", &s, &s]);
            }
        }
    }
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

    // Environment allowlist (see ENV_PASSTHROUGH).
    for key in ENV_PASSTHROUGH {
        if let Some(v) = std::env::var_os(key).and_then(|v| v.into_string().ok()) {
            cmd.args(["--setenv", key, &v]);
        }
    }
    for (k, v) in std::env::vars_os() {
        if let (Some(k), Some(v)) = (k.to_str(), v.to_str()) {
            if k.starts_with("LC_") {
                cmd.args(["--setenv", k, v]);
            }
        }
    }
    if net {
        for key in ENV_PROXY {
            if let Some(v) = std::env::var_os(key).and_then(|v| v.into_string().ok()) {
                cmd.args(["--setenv", key, &v]);
            }
        }
    }

    // Isolated public keyring: fresh GNUPGHOME, seed from host export,
    // then recv any explicit keys (validpgpkeys / log-cited). Bound
    // read-only so the build cannot plant keys or touch the real home.
    if let Some(gnupg) = sandbox_gnupg_home(build_dir) {
        if let Some(real) = real_gnupg_home {
            seed_from_host_export(real, &gnupg);
        }
        if !extra_pgp_keys.is_empty() {
            let _ = sandbox_recv_keys(&gnupg, extra_pgp_keys, "keyserver.ubuntu.com");
        }
        let dest = fake_home.join(".gnupg");
        let dest_s = dest.to_string_lossy().to_string();
        let src_s = gnupg.to_string_lossy().to_string();
        cmd.args(["--ro-bind", &src_s, &dest_s]);
    }

    // rustup: see rustup_env's doc comment.
    let rust = rustup_env(build_dir);
    for p in &rust.ro_binds {
        let s = p.to_string_lossy().to_string();
        cmd.args(["--ro-bind", &s, &s]);
    }
    for (k, v) in &rust.env {
        cmd.args(["--setenv", k, v]);
    }

    // PATH (explicit now that the environment is cleared), with the
    // fakeroot shim in front: see fakeroot_shim_dir's doc comment.
    let host_path =
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    let sandboxed_path = match fakeroot_shim_dir(build_dir, extra_dest_dirs) {
        Some(shim_dir) => format!("{}:{}", shim_dir.display(), host_path),
        None => host_path,
    };
    cmd.args(["--setenv", "PATH", &sandboxed_path]);

    cmd.args(["--chdir", &build_dir_s]);
    cmd.arg("--");
    if net {
        cmd.arg(makepkg_bin);
        cmd.args(caller_args);
    } else {
        // With `net: false` the namespace's own `lo` starts DOWN. Some
        // fakeroot builds use TCP-loopback IPC, which needs it. Bring it
        // up first -- loopback only, still no outside route. "$0" "$@"

        cmd.arg("/bin/sh");
        cmd.args([
            "-c",
            "ip link set lo up >/dev/null 2>&1; exec \"$0\" \"$@\"",
        ]);
        cmd.arg(makepkg_bin);
        cmd.args(caller_args);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn temp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("ae-sandbox-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scratch_dir_is_private_random_cached_and_cleaned() {
        let a = Path::new("/nonexistent/ae-build-a");
        let b = Path::new("/nonexistent/ae-build-b");
        let da = fakeroot_shim_scratch_dir(a);
        assert_eq!(da, fakeroot_shim_scratch_dir(a));
        let db = fakeroot_shim_scratch_dir(b);
        assert_ne!(da, db);
        assert_eq!(std::fs::metadata(&da).unwrap().mode() & 0o777, 0o700);
        drop(FakerootShimGuard::new(a));
        drop(FakerootShimGuard::new(b));
        assert!(!da.exists() && !db.exists());
    }

    #[test]
    fn shim_skips_bwrap_for_version_probe_and_when_already_in_fakeroot() {
        let dir = temp("shim");
        let real = dir.join("real-fakeroot");
        std::fs::write(&real, "#!/bin/sh\necho REAL \"$@\"\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_shim(&dir, &real, "--ro-bind / /").unwrap();
        let shim = dir.join("fakeroot");

        // `fakeroot -v` (makepkg's .PKGINFO stamp): must not touch bwrap.
        let out = Command::new(&shim)
            .arg("-v")
            .env_remove("FAKEROOTKEY")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "REAL -v\n");

        // Already inside a fakeroot: pass straight through.
        let out = Command::new(&shim)
            .args(["--", "true"])
            .env("FAKEROOTKEY", "123")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "REAL -- true\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shim_write_does_not_follow_a_planted_symlink() {
        let dir = temp("symlink");
        let victim = dir.join("victim");
        std::fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join(".fakeroot.new")).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("fakeroot")).unwrap();
        write_shim(&dir, Path::new("/usr/bin/fakeroot"), "--ro-bind / /").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert!(std::fs::read_to_string(dir.join("fakeroot"))
            .unwrap()
            .starts_with("#!/bin/sh"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn is_real_dir_rejects_missing_and_symlinks() {
        let d = temp("realdir");
        std::os::unix::fs::symlink(&d, d.join("link")).unwrap();
        assert!(is_real_dir(&d));
        assert!(!is_real_dir(&d.join("nope")));
        assert!(!is_real_dir(&d.join("link")));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn config_symlinked_into_hidden_dir_is_reexposed() {
        let root = temp("cfglink");
        let home = root.join("home");
        std::fs::create_dir_all(home.join("dots")).unwrap();
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(home.join("dots/makepkg.conf"), "PKGEXT='.pkg.tar.zst'").unwrap();
        std::fs::write(root.join("etc/plain.conf"), "x").unwrap();
        std::os::unix::fs::symlink(
            home.join("dots/makepkg.conf"),
            root.join("etc/makepkg.conf"),
        )
        .unwrap();
        let real_home = std::fs::canonicalize(&home).unwrap();
        let cands = vec![
            root.join("etc/makepkg.conf"),
            root.join("etc/plain.conf"),
            root.join("etc/missing.conf"),
        ];
        let got = exposed_targets(&cands, &[real_home.clone()]);
        assert_eq!(got, vec![real_home.join("dots/makepkg.conf")]);
        assert!(exposed_targets(&cands, &[PathBuf::from("/nonexistent-hidden")]).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sandbox_gnupg_home_is_empty_ring() {
        let bd = Path::new("/nonexistent/ae-build-gnupg");
        let copy = sandbox_gnupg_home(bd).unwrap();
        assert!(copy.is_dir());
        assert!(copy.join("gpg.conf").is_file());
        assert!(!copy.join("private-keys-v1.d").exists());
        drop(FakerootShimGuard::new(bd));
    }
}
