//! Keyring populate without `pacman-key`: same steps, `gpg` called
//! directly. Std + libc only, no `crate::` imports.
//!
//! Per keyring `<name>` (files in KEYRING_DIR):
//!   import `<name>.gpg` -> lsign ids from `<name>-trusted` ->
//!   import ownertrust -> disable ids from `<name>-revoked`;
//! then one `--check-trustdb`.

use std::fs;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

const GPG: &str = "/usr/bin/gpg";
const HOMEDIR: &str = "/etc/pacman.d/gnupg";
const KEYRING_DIR: &str = "/usr/share/pacman/keyrings";

fn fail(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// Hex key id / fingerprint (16..=40 digits).
fn is_fpr(s: &str) -> bool {
    (16..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Keyring name from a file name: `<name>.gpg`, safe chars only.
fn keyring_name(file: &str) -> Option<String> {
    let n = file.strip_suffix(".gpg")?;
    let ok = !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    ok.then(|| n.to_string())
}

/// Packaged keyrings (archlinux, artix, ...), sorted.
pub(crate) fn names() -> Vec<String> {
    let Ok(rd) = fs::read_dir(KEYRING_DIR) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .filter_map(|e| keyring_name(e.file_name().to_str()?))
        .collect();
    v.sort();
    v
}

/// `<name>-trusted`: lines `FPR:LEVEL:`. Returns (ids, valid lines).
fn parse_trusted(text: &str) -> (Vec<String>, Vec<String>) {
    let mut ids = Vec::new();
    let mut lines = Vec::new();
    for l in text.lines().map(str::trim) {
        let id = l.split(':').next().unwrap_or("");
        if l.starts_with('#') || !is_fpr(id) {
            continue;
        }
        ids.push(id.to_string());
        lines.push(l.to_string());
    }
    (ids, lines)
}

/// `<name>-revoked`: one id per line.
fn parse_revoked(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && is_fpr(l))
        .map(str::to_string)
        .collect()
}

/// One gpg run. Returns stdout; Err carries the last stderr line.
fn gpg(args: &[&str], input: Option<&str>) -> io::Result<String> {
    let mut cmd = Command::new(GPG);
    cmd.args([
        "--homedir",
        HOMEDIR,
        "--no-permission-warning",
        "--quiet",
        "--batch",
    ])
    .args(args)
    .env("LC_ALL", "C")
    .stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    // The helper runs with umask 077; gpg files must stay readable.
    // SAFETY: umask() is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::umask(0o022);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| fail(format!("{}: {}", GPG, e)))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(text.as_bytes());
    }
    let out = child.wait_with_output()?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let last = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("failed");
    Err(fail(format!(
        "gpg {}: {}",
        args.first().unwrap_or(&"?"),
        last.trim()
    )))
}

/// lsign needs the local master key (`pacman-key --init`).
fn has_master_key() -> bool {
    gpg(&["--list-secret-keys", "--with-colons"], None)
        .map(|o| o.lines().any(|l| l.starts_with("sec:")))
        .unwrap_or(false)
}

/// Missing file = empty.
fn read_opt(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Returns the number of ids that could not be signed / disabled.
fn populate_one(name: &str) -> io::Result<usize> {
    let base = format!("{}/{}", KEYRING_DIR, name);
    gpg(&["--import", &format!("{}.gpg", base)], None)?;

    let (ids, lines) = parse_trusted(&read_opt(&format!("{}-trusted", base)));
    let mut bad = 0;
    for id in &ids {
        if gpg(&["--yes", "--lsign-key", id], None).is_err() {
            bad += 1;
        }
    }
    if !lines.is_empty() {
        gpg(&["--import-ownertrust"], Some(&(lines.join("\n") + "\n")))?;
    }
    for id in parse_revoked(&read_opt(&format!("{}-revoked", base))) {
        let r = gpg(
            &["--command-fd", "0", "--edit-key", &id],
            Some("disable\nsave\n"),
        );
        if r.is_err() {
            bad += 1;
        }
    }
    Ok(bad)
}

/// `pacman-key --populate <all packaged keyrings>`.
/// `log` gets one short line per step.
pub(crate) fn populate_all(log: &mut dyn FnMut(&str)) -> io::Result<()> {
    let names = names();
    if names.is_empty() {
        return Err(fail(format!("no keyrings in {}", KEYRING_DIR)));
    }
    if !has_master_key() {
        return Err(fail(
            "pacman keyring is not initialised: run `pacman-key --init`",
        ));
    }
    let mut done = 0;
    for n in &names {
        match populate_one(n) {
            Ok(0) => {
                done += 1;
                log(&format!("Keyring: {} ok", n));
            }
            Ok(bad) => {
                done += 1;
                log(&format!("Keyring: {} ok, {} key(s) skipped", n, bad));
            }
            Err(e) => log(&format!("Keyring: {} failed: {}", n, e)),
        }
    }
    if done == 0 {
        return Err(fail("no keyring could be imported"));
    }
    gpg(&["--check-trustdb"], None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_file() {
        assert_eq!(keyring_name("artix.gpg").as_deref(), Some("artix"));
        assert_eq!(keyring_name("artix-trusted"), None);
        assert_eq!(keyring_name("a b.gpg"), None);
        assert_eq!(keyring_name("../x.gpg"), None);
        assert_eq!(keyring_name(".gpg"), None);
    }

    #[test]
    fn trusted_lines() {
        let t =
            "# c\n\n1247D995F165BBAC:4:\nbad:4:\n  0123456789ABCDEF0123456789ABCDEF01234567:5:  \n";
        let (ids, lines) = parse_trusted(t);
        assert_eq!(
            ids,
            [
                "1247D995F165BBAC",
                "0123456789ABCDEF0123456789ABCDEF01234567"
            ]
        );
        assert_eq!(lines[0], "1247D995F165BBAC:4:");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn revoked_lines() {
        let t = "# x\n1247D995F165BBAC\nzz\n\n  0123456789ABCDEF  \n";
        assert_eq!(parse_revoked(t), ["1247D995F165BBAC", "0123456789ABCDEF"]);
    }
}
