//! `FEATURES="candy"` and `--moo`. Cosmetic only, never touches stdout
//! (the spinner draws on stderr, so pipes stay clean).

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const BANNER: &str = "Gentoo Rocks (Arch)";
/// Visible width of the scrolling window.
const WIDTH: usize = 20;
const TICK: Duration = Duration::from_millis(80);

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

/// `emerge --moo`.
pub(crate) fn moo() {
    println!("{}", MOO.trim_matches('\n'));
}

/// Frame `tick` of the marquee: BANNER scrolling through a WIDTH window.
pub(crate) fn scroll_frame(tick: usize) -> String {
    let cycle: Vec<char> = BANNER
        .chars()
        .chain(std::iter::repeat(' ').take(WIDTH))
        .collect();
    (0..WIDTH)
        .map(|i| cycle[(tick + i) % cycle.len()])
        .collect()
}

/// Pure part of the on/off decision.
fn wanted(candy: bool, tty: bool, term: Option<&str>) -> bool {
    candy && tty && term.map_or(true, |t| t != "dumb")
}

/// Marquee on stderr while it lives; the line is wiped on drop.
/// Not called yet: goes around the real resolver work.
#[allow(dead_code)]
pub(crate) struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[allow(dead_code)]
impl Spinner {
    /// `None` unless candy is on (`--nospinner`/`-q` already folded into
    /// `crate::runtime::candy()`) and stderr is a real terminal.
    pub(crate) fn start() -> Option<Spinner> {
        let term = std::env::var("TERM").ok();
        if !wanted(
            crate::runtime::candy(),
            std::io::stderr().is_terminal(),
            term.as_deref(),
        ) {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut tick = 0usize;
            while !flag.load(Ordering::Relaxed) {
                let mut err = std::io::stderr().lock();
                let _ = write!(err, "\r{}", scroll_frame(tick));
                let _ = err.flush();
                drop(err);
                tick = tick.wrapping_add(1);
                thread::sleep(TICK);
            }
        });
        Some(Spinner {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r\x1b[K");
        let _ = err.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_have_fixed_width() {
        for t in 0..100 {
            assert_eq!(scroll_frame(t).chars().count(), WIDTH);
        }
    }

    #[test]
    fn marquee_scrolls_and_wraps() {
        assert!(scroll_frame(0).starts_with("Gentoo Rocks"));
        assert!(scroll_frame(1).starts_with("entoo Rocks"));
        let period = BANNER.chars().count() + WIDTH;
        assert_eq!(scroll_frame(3), scroll_frame(3 + period));
    }

    #[test]
    fn needs_candy_and_a_real_terminal() {
        assert!(wanted(true, true, Some("xterm-256color")));
        assert!(wanted(true, true, None));
        assert!(!wanted(false, true, Some("xterm")));
        assert!(!wanted(true, false, Some("xterm")));
        assert!(!wanted(true, true, Some("dumb")));
    }

    #[test]
    fn moo_says_it() {
        assert!(MOO.contains("Moo!"));
        assert!(MOO.lines().all(|l| l.chars().count() < 60));
    }
}
