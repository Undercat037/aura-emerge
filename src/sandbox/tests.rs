//! Unit tests for `sandbox` (kept out of the module file so the code stays readable).

use super::*;
use std::os::unix::fs::MetadataExt;

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("ae-sandbox-test-{}-{}", name, std::process::id()));
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
fn keyring_copy_has_public_files_only() {
    let real = temp("gnupg-real");
    std::fs::write(real.join("pubring.kbx"), "pub").unwrap();
    std::fs::write(real.join("trustdb.gpg"), "trust").unwrap();
    std::fs::create_dir_all(real.join("private-keys-v1.d")).unwrap();
    std::fs::write(real.join("private-keys-v1.d/k.key"), "SECRET").unwrap();
    let bd = Path::new("/nonexistent/ae-build-gnupg");
    let copy = public_only_gnupg(&real, bd).unwrap();
    assert!(copy.join("pubring.kbx").is_file());
    assert!(copy.join("trustdb.gpg").is_file());
    assert!(!copy.join("private-keys-v1.d").exists());
    drop(FakerootShimGuard::new(bd));
    let _ = std::fs::remove_dir_all(real);
}
