//! Message colours from `COLORS` in make.conf.
//!
//! The code paints with five fixed colours (green, yellow, red, cyan,
//! magenta). Each one is a role here, and `COLORS` can swap it for a
//! HEX value or an ANSI colour name:
//!
//! ```text
//! COLORS="ok=#a6e3a1 warn=#f9e2af error=#f38ba8 info=#89b4fa special=#cba6f7"
//! ```
//!
//! `ok` = green, `warn` = yellow, `error` = red, `info` = cyan,
//! `special` = magenta. Unset roles keep their usual colour, and
//! `--color=n` / `NO_COLOR` still turn everything off.

use std::sync::OnceLock;

use colored::{Color, ColoredString, Colorize};

#[derive(Default)]
struct Palette {
    ok: Option<Color>,
    warn: Option<Color>,
    error: Option<Color>,
    info: Option<Color>,
    special: Option<Color>,
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

/// Called once after make.conf is read. A second call is ignored.
pub(crate) fn init(colors: &[(String, Color)]) {
    let mut p = Palette::default();
    for (role, c) in colors {
        let slot = match role.as_str() {
            "ok" => &mut p.ok,
            "warn" => &mut p.warn,
            "error" => &mut p.error,
            "info" => &mut p.info,
            "special" => &mut p.special,
            _ => continue,
        };
        *slot = Some(c.clone());
    }
    let _ = PALETTE.set(p);
}

fn paint(s: ColoredString, pick: fn(&Palette) -> Option<Color>, default: Color) -> ColoredString {
    let c = PALETTE.get().and_then(pick).unwrap_or(default);
    s.color(c)
}

/// Same as `colored`'s `.green()` and friends, but through the palette.
pub(crate) trait Themed: Sized {
    fn t_green(self) -> ColoredString;
    fn t_yellow(self) -> ColoredString;
    fn t_red(self) -> ColoredString;
    fn t_cyan(self) -> ColoredString;
    fn t_magenta(self) -> ColoredString;
}

macro_rules! themed_impl {
    ($t:ty, $to:expr) => {
        impl Themed for $t {
            fn t_green(self) -> ColoredString {
                paint($to(self), |p| p.ok.clone(), Color::Green)
            }
            fn t_yellow(self) -> ColoredString {
                paint($to(self), |p| p.warn.clone(), Color::Yellow)
            }
            fn t_red(self) -> ColoredString {
                paint($to(self), |p| p.error.clone(), Color::Red)
            }
            fn t_cyan(self) -> ColoredString {
                paint($to(self), |p| p.info.clone(), Color::Cyan)
            }
            fn t_magenta(self) -> ColoredString {
                paint($to(self), |p| p.special.clone(), Color::Magenta)
            }
        }
    };
}

themed_impl!(&str, |s: &str| ColoredString::from(s));
themed_impl!(ColoredString, |s: ColoredString| s);
