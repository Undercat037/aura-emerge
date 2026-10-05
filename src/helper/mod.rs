//! Root helper (`--ae-service`). Built up piece by piece.
//! Rule: no imports from the rest of the crate; only std, libc and siblings.

// Until main() calls into it.
#![allow(dead_code)]

pub(crate) mod client;
pub(crate) mod fsio;
pub(crate) mod guard;
pub(crate) mod pkgdb;
pub(crate) mod pkgmask;
pub(crate) mod proto;
pub(crate) mod serve;
pub(crate) mod sha256;
pub(crate) mod stage;
pub(crate) mod validate;

use std::fs::File;
use std::io::BufReader;
use std::os::fd::FromRawFd;

/// Entry for `--ae-service`; returns the process exit code.
/// Call before clap, config and runtime are touched.
pub(crate) fn run() -> i32 {
    if let Err(e) = guard::harden() {
        eprintln!("emerge helper: {}", e);
        return 1;
    }
    // SAFETY: harden() checked fd 3/4 are open pipes/sockets and closed
    // everything above; we take ownership exactly once, here.
    let (inp, mut out) = unsafe {
        (
            File::from_raw_fd(guard::FD_IN),
            File::from_raw_fd(guard::FD_OUT),
        )
    };
    match serve::serve(&mut BufReader::new(inp), &mut out, &mut serve::Real) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("emerge helper: {}", e);
            1
        }
    }
}
