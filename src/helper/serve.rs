//! Helper main loop: request in, response out, one at a time.
//! Std only (plus `proto`/`fsio`/`validate`), no `crate::` imports.
//!
//! The loop knows nothing about pacman: all real work sits behind
//! `Backend`, so it can be tested with a mock.

use std::io::{self, BufRead, ErrorKind, Write};

use super::fsio;
use super::pkgdb;
use super::proto::{self, ProtoError, Request, Response};
use super::validate::{self, FileOpts, RemoveMode, Target};

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(ErrorKind::Unsupported, "not implemented"))
}

/// What the helper can actually do. Unimplemented verbs fall through
/// to the defaults, so adding one = override one method.
pub(crate) trait Backend {
    fn append(&mut self, target: Target, line: &str) -> io::Result<()>;

    fn sync(&mut self, _force: bool, _emit: &mut dyn FnMut(&str)) -> io::Result<()> {
        unsupported()
    }
    fn sysupgrade(&mut self, _ignore: &[String], _emit: &mut dyn FnMut(&str)) -> io::Result<()> {
        unsupported()
    }
    fn install(&mut self, _names: &[String], _needed: bool) -> io::Result<()> {
        unsupported()
    }
    fn install_files(&mut self, _opts: FileOpts, _specs: &[String]) -> io::Result<()> {
        unsupported()
    }
    fn remove(&mut self, _mode: RemoveMode, _names: &[String]) -> io::Result<()> {
        unsupported()
    }
    fn set_reason(&mut self, _explicit: bool, _names: &[String]) -> io::Result<()> {
        unsupported()
    }
}

/// The real thing.
pub(crate) struct Real;

impl Backend for Real {
    fn append(&mut self, target: Target, line: &str) -> io::Result<()> {
        fsio::append(target, line)
    }

    fn sync(&mut self, force: bool, emit: &mut dyn FnMut(&str)) -> io::Result<()> {
        pkgdb::sync(force, emit)
    }

    fn sysupgrade(&mut self, ignore: &[String], emit: &mut dyn FnMut(&str)) -> io::Result<()> {
        pkgdb::sysupgrade(ignore, emit)
    }

    fn install(&mut self, names: &[String], needed: bool) -> io::Result<()> {
        pkgdb::install(names, needed)
    }

    fn install_files(&mut self, opts: FileOpts, specs: &[String]) -> io::Result<()> {
        pkgdb::install_files(opts, specs)
    }

    fn set_reason(&mut self, explicit: bool, names: &[String]) -> io::Result<()> {
        pkgdb::set_reason(explicit, names)
    }

    fn remove(&mut self, mode: RemoveMode, names: &[String]) -> io::Result<()> {
        pkgdb::remove(mode, names)
    }
}

fn dispatch<B: Backend, W: Write>(be: &mut B, req: &Request, out: &mut W) -> io::Result<()> {
    match req {
        Request::Ping | Request::Quit => Ok(()),
        Request::Sync { force } => be.sync(*force, &mut |t| {
            // Best effort: a dead peer shows up on the final write.
            let _ = proto::write_response(out, &Response::Event(t.to_string()));
        }),
        Request::Sysupgrade { ignore } => be.sysupgrade(ignore, &mut |t| {
            let _ = proto::write_response(out, &Response::Event(t.to_string()));
        }),
        Request::Install { names, needed } => be.install(names, *needed),
        Request::InstallFiles { opts, files } => be.install_files(*opts, files),
        Request::Remove { mode, names } => be.remove(*mode, names),
        Request::SetReason { explicit, names } => be.set_reason(*explicit, names),
        Request::Append { target, line } => be.append(*target, line),
    }
}

/// Error text for the wire: one short clean line.
fn clean(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect();
    validate::line(t.trim())
        .map(str::to_string)
        .unwrap_or_else(|_| "error".to_string())
}

/// `Ok` = client hung up or sent `quit`. `Err` = stream is broken.
pub(crate) fn serve<R: BufRead, W: Write, B: Backend>(
    input: &mut R,
    out: &mut W,
    be: &mut B,
) -> Result<(), ProtoError> {
    loop {
        let req = match proto::read_request(input) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()),
            Err(e) => {
                // Best effort: the peer may already be gone.
                let _ = proto::write_response(out, &Response::Fail(clean(&e.to_string())));
                if e.is_fatal() {
                    return Err(e);
                }
                continue;
            }
        };
        let resp = match dispatch(be, &req, out) {
            Ok(()) => Response::Done,
            Err(e) => Response::Fail(clean(&e.to_string())),
        };
        proto::write_response(out, &resp)?;
        if req == Request::Quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mock {
        appended: Vec<(Target, String)>,
        reasons: Vec<(bool, Vec<String>)>,
        removed: Vec<(RemoveMode, Vec<String>)>,
        synced: Vec<bool>,
        installed: Vec<Vec<String>>,
        files: Vec<(FileOpts, Vec<String>)>,
        fail: bool,
    }

    impl Backend for Mock {
        fn append(&mut self, target: Target, line: &str) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.appended.push((target, line.to_string()));
            Ok(())
        }

        fn sync(&mut self, force: bool, _emit: &mut dyn FnMut(&str)) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.synced.push(force);
            Ok(())
        }

        fn install(&mut self, names: &[String], _needed: bool) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.installed.push(names.to_vec());
            Ok(())
        }

        fn install_files(&mut self, opts: FileOpts, specs: &[String]) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.files.push((opts, specs.to_vec()));
            Ok(())
        }

        fn set_reason(&mut self, explicit: bool, names: &[String]) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.reasons.push((explicit, names.to_vec()));
            Ok(())
        }

        fn remove(&mut self, mode: RemoveMode, names: &[String]) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(ErrorKind::Other, "boom"));
            }
            self.removed.push((mode, names.to_vec()));
            Ok(())
        }
    }

    fn run(input: &str, be: &mut Mock) -> (Result<(), ProtoError>, String) {
        let mut out = Vec::new();
        let res = serve(&mut input.as_bytes(), &mut out, be);
        (res, String::from_utf8(out).unwrap())
    }

    #[test]
    fn ping_then_quit() {
        let (res, out) = run("CMD ping\nEND\nCMD quit\nEND\n", &mut Mock::default());
        assert!(res.is_ok());
        assert_eq!(out, "OK\nOK\n");
    }

    #[test]
    fn nothing_after_quit_is_run() {
        let (res, out) = run("CMD quit\nEND\nCMD ping\nEND\n", &mut Mock::default());
        assert!(res.is_ok());
        assert_eq!(out, "OK\n");
    }

    #[test]
    fn client_hangup_is_clean() {
        let (res, out) = run("", &mut Mock::default());
        assert!(res.is_ok());
        assert_eq!(out, "");
    }

    #[test]
    fn append_reaches_backend() {
        let mut be = Mock::default();
        let (res, out) = run("CMD append\nARG log\nARG hello\nEND\n", &mut be);
        assert!(res.is_ok());
        assert_eq!(out, "OK\n");
        assert_eq!(be.appended, vec![(Target::EmergeLog, "hello".to_string())]);
    }

    #[test]
    fn sync_reaches_backend() {
        let mut be = Mock::default();
        let (res, out) = run("CMD sync\nEND\nCMD refresh\nEND\n", &mut be);
        assert!(res.is_ok());
        assert_eq!(out, "OK\nOK\n");
        assert_eq!(be.synced, vec![false, true]);
    }

    #[test]
    fn install_reaches_backend() {
        let mut be = Mock::default();
        let (res, out) = run("CMD install\nARG nano\nARG extra/vim\nEND\n", &mut be);
        assert!(res.is_ok());
        assert_eq!(out, "OK\n");
        assert_eq!(
            be.installed,
            vec![vec!["nano".to_string(), "extra/vim".to_string()]]
        );
    }

    #[test]
    fn install_files_reaches_backend_and_bad_spec_does_not() {
        let mut be = Mock::default();
        let spec = format!("{} /tmp/a.pkg.tar.zst", "a".repeat(64));
        let input = format!(
            "CMD installfile\nARG needed,asdeps\nARG {}\nEND\n\
             CMD installfile\nARG -\nARG /etc/shadow\nEND\n",
            spec
        );
        let (res, out) = run(&input, &mut be);
        assert!(res.is_ok());
        assert!(out.starts_with("OK\nERR "), "{}", out);
        let opts = FileOpts {
            needed: true,
            asdeps: true,
        };
        assert_eq!(be.files, vec![(opts, vec![spec])]);
    }

    #[test]
    fn set_reason_reaches_backend() {
        let mut be = Mock::default();
        let (res, out) = run(
            "CMD asexplicit\nARG nano\nARG extra/vim\nEND\nCMD asdeps\nARG foo\nEND\n",
            &mut be,
        );
        assert!(res.is_ok());
        assert_eq!(out, "OK\nOK\n");
        assert_eq!(
            be.reasons,
            vec![
                (true, vec!["nano".to_string(), "extra/vim".to_string()]),
                (false, vec!["foo".to_string()]),
            ]
        );
    }

    #[test]
    fn remove_modes_reach_backend() {
        let mut be = Mock::default();
        let (res, out) = run(
            "CMD remove\nARG a\nEND\nCMD unmerge\nARG b\nARG c\nEND\nCMD prune\nARG d\nEND\n",
            &mut be,
        );
        assert!(res.is_ok());
        assert_eq!(out, "OK\nOK\nOK\n");
        assert_eq!(
            be.removed,
            vec![
                (RemoveMode::Plain, vec!["a".to_string()]),
                (RemoveMode::Unmerge, vec!["b".to_string(), "c".to_string()]),
                (RemoveMode::Prune, vec!["d".to_string()]),
            ]
        );
    }

    #[test]
    fn backend_error_is_reported_and_loop_continues() {
        let mut be = Mock {
            fail: true,
            ..Default::default()
        };
        let (res, out) = run("CMD append\nARG log\nARG x\nEND\nCMD ping\nEND\n", &mut be);
        assert!(res.is_ok());
        assert_eq!(out, "ERR boom\nOK\n");
    }

    #[test]
    fn unimplemented_verb_fails_softly() {
        // Only `append` implemented: the rest hits the defaults.
        struct Bare;
        impl Backend for Bare {
            fn append(&mut self, _: Target, _: &str) -> io::Result<()> {
                Ok(())
            }
        }
        let mut input = "CMD install\nARG nano\nEND\nCMD sync\nEND\nCMD ping\nEND\n".as_bytes();
        let mut out = Vec::new();
        let res = serve(&mut input, &mut out, &mut Bare);
        assert!(res.is_ok());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "ERR not implemented\nERR not implemented\nOK\n"
        );
    }

    #[test]
    fn bad_content_is_recoverable() {
        let (res, out) = run(
            "CMD rm\nEND\nCMD install\nARG --noconfirm\nEND\nCMD ping\nEND\n",
            &mut Mock::default(),
        );
        assert!(res.is_ok());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("ERR "));
        assert!(lines[1].starts_with("ERR "));
        assert_eq!(lines[2], "OK");
    }

    #[test]
    fn framing_error_stops_the_helper() {
        let mut be = Mock::default();
        let (res, out) = run("garbage\nCMD append\nARG log\nARG x\nEND\n", &mut be);
        assert!(res.is_err());
        assert_eq!(out.lines().count(), 1);
        assert!(out.starts_with("ERR "));
        assert!(be.appended.is_empty());
    }

    #[test]
    fn truncated_request_is_an_error() {
        let (res, _) = run("CMD append\nARG log\n", &mut Mock::default());
        assert!(matches!(res, Err(ProtoError::Eof)));
    }

    #[test]
    fn error_text_is_one_clean_line() {
        assert_eq!(clean("a\nb\x1b[2J"), "a b [2J");
        assert_eq!(clean("\n\n"), "error");
        assert_eq!(clean(&"x".repeat(500)).len(), 200);
    }
}
