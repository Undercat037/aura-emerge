//! `--moo` and the Portage-style "Calculating dependencies" lines.
//! Cosmetic only; the lines go to stdout, nothing animates.

use std::sync::Mutex;
use std::time::Instant;

const MOO: &str = r#"

  Larry loves Gentoo (Linux)

 _______________________
< Have you mooed today? >
 -----------------------
        \   ^__^
         \  (oo)\_______
            (__)\       )\/\\
                ||----w |
                ||     ||
"#;

/// Start of the current resolve phase (run start, reset after a db sync
/// so network time is not counted as resolution).
static MARK: Mutex<Option<Instant>> = Mutex::new(None);

/// `emerge --moo`.
pub(crate) fn moo() {
    println!("{}", MOO.trim_matches('\n'));
}

/// Begin timing the resolve phase.
pub(crate) fn mark_start() {
    if let Ok(mut m) = MARK.lock() {
        *m = Some(Instant::now());
    }
}

fn took_line(secs: f64) -> String {
    format!("Dependency resolution took {:.2} s", secs)
}

/// "Calculating dependencies ... done!" only.
pub(crate) fn calculating_deps_line() {
    println!("Calculating dependencies ... done!");
}

/// The line above plus how long resolution took since `mark_start`.
pub(crate) fn calculating_deps_done() {
    calculating_deps_line();
    let secs = MARK
        .lock()
        .ok()
        .and_then(|m| m.map(|t| t.elapsed().as_secs_f64()))
        .unwrap_or(0.0);
    println!("{}", took_line(secs));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn took_line_has_two_decimals() {
        assert_eq!(took_line(1.666), "Dependency resolution took 1.67 s");
        assert_eq!(took_line(0.0), "Dependency resolution took 0.00 s");
    }

    #[test]
    fn moo_says_it() {
        assert!(MOO.contains("Have you mooed today?"));
        assert!(MOO.lines().all(|l| l.chars().count() < 60));
    }
}
