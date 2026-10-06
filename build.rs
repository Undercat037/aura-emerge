// Exposes the locked `alpm` crate version as AE_ALPM_CRATE_VERSION.
fn main() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let ver = std::fs::read_to_string("Cargo.lock")
        .ok()
        .and_then(|t| {
            let mut lines = t.lines();
            while let Some(l) = lines.next() {
                if l.trim() == "name = \"alpm\"" {
                    let v = lines.next()?;
                    return v
                        .trim()
                        .strip_prefix("version = \"")
                        .and_then(|s| s.strip_suffix('"'))
                        .map(str::to_string);
                }
            }
            None
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AE_ALPM_CRATE_VERSION={ver}");
}
