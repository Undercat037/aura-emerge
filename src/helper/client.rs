//! Client side: starts the root helper through sudo and talks to it.
//! Std only, no `crate::` imports.
//!
//! One sudo prompt per run: the helper stays up and serves every
//! request until `quit` / EOF (this replaces `--sudoloop`).
//! The channel is the child's stdin/stdout; stderr stays on the
//! terminal so sudo's prompt and helper diagnostics remain visible.

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use super::guard::EXPECTED_EXE;
use super::proto::{self, ProtoError, Request, Response};
use super::validate::{FileOpts, RemoveMode, Target};

const SUDO_BIN: &str = "/usr/bin/sudo";

#[derive(Debug)]
pub(crate) enum ClientError {
    Spawn(io::Error),
    /// sudo denied, or the helper refused to run (its reason is on stderr).
    NoHelper,
    Proto(ProtoError),
    /// Helper understood the request and said no.
    Refused(String),
    /// A previous error left the stream unusable.
    Broken,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Spawn(e) => write!(f, "cannot start sudo: {}", e),
            ClientError::NoHelper => write!(f, "root helper did not start"),
            ClientError::Proto(e) => write!(f, "helper protocol error: {}", e),
            ClientError::Refused(m) => write!(f, "helper refused: {}", m),
            ClientError::Broken => write!(f, "helper connection is broken"),
        }
    }
}

impl From<ProtoError> for ClientError {
    fn from(e: ProtoError) -> Self {
        ClientError::Proto(e)
    }
}

pub(crate) struct Client {
    child: Option<Child>,
    rx: Box<dyn BufRead + Send>,
    tx: Box<dyn Write + Send>,
    broken: bool,
}

impl Client {
    /// `sudo /usr/bin/emerge --ae-service`, then a ping handshake so
    /// a denied password fails here and not on the first real request.
    pub(crate) fn start() -> Result<Client, ClientError> {
        Self::spawn(SUDO_BIN, &[EXPECTED_EXE, "--ae-service"])
    }

    fn spawn(prog: &str, args: &[&str]) -> Result<Client, ClientError> {
        let mut child = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(ClientError::Spawn)?;
        let (Some(tx), Some(rx)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClientError::NoHelper);
        };
        let mut c = Client::from_streams(Box::new(BufReader::new(rx)), Box::new(tx), Some(child));
        match c.ping() {
            Ok(()) => Ok(c),
            Err(ClientError::Proto(e)) if e.is_fatal() => Err(ClientError::NoHelper),
            Err(e) => Err(e),
        }
    }

    fn from_streams(
        rx: Box<dyn BufRead + Send>,
        tx: Box<dyn Write + Send>,
        child: Option<Child>,
    ) -> Client {
        Client {
            child,
            rx,
            tx,
            broken: false,
        }
    }

    /// Sends one request; `on_event` gets progress lines until the
    /// final OK / ERR. A fatal stream error poisons the client.
    pub(crate) fn request(
        &mut self,
        req: &Request,
        on_event: &mut dyn FnMut(&str),
    ) -> Result<(), ClientError> {
        if self.broken {
            return Err(ClientError::Broken);
        }
        match self.exchange(req, on_event) {
            Err(ClientError::Proto(e)) if e.is_fatal() => {
                self.broken = true;
                Err(ClientError::Proto(e))
            }
            other => other,
        }
    }

    fn exchange(
        &mut self,
        req: &Request,
        on_event: &mut dyn FnMut(&str),
    ) -> Result<(), ClientError> {
        proto::write_request(&mut self.tx, req)?;
        loop {
            match proto::read_response(&mut self.rx)? {
                Response::Event(t) => on_event(&t),
                Response::Done => return Ok(()),
                Response::Fail(m) => return Err(ClientError::Refused(m)),
            }
        }
    }

    pub(crate) fn ping(&mut self) -> Result<(), ClientError> {
        self.request(&Request::Ping, &mut |_| {})
    }

    pub(crate) fn append(&mut self, target: Target, line: &str) -> Result<(), ClientError> {
        let req = Request::Append {
            target,
            line: line.to_string(),
        };
        self.request(&req, &mut |_| {})
    }

    /// Removes installed packages; `mode` picks the pacman -R flavour.
    pub(crate) fn remove(&mut self, mode: RemoveMode, names: &[String]) -> Result<(), ClientError> {
        let req = Request::Remove {
            mode,
            names: names.to_vec(),
        };
        self.request(&req, &mut |_| {})
    }

    /// `pacman -S`: install from sync dbs by `[repo/]name`.
    pub(crate) fn install(&mut self, names: &[String], needed: bool) -> Result<(), ClientError> {
        let req = Request::Install {
            needed,
            names: names.to_vec(),
        };
        self.request(&req, &mut |_| {})
    }

    /// Refresh sync dbs (`pacman -Sy`); `force` is `-Syy`.
    /// `on_event` gets `sync <repo> <updated|uptodate|failed>` per db.
    pub(crate) fn sync(
        &mut self,
        force: bool,
        on_event: &mut dyn FnMut(&str),
    ) -> Result<(), ClientError> {
        self.request(&Request::Sync { force }, on_event)
    }

    /// Full official upgrade (`pacman -Su`); `ignore` is `--ignore` names.
    /// `on_event` gets `pkg start|done <name>` per package.
    pub(crate) fn sysupgrade(
        &mut self,
        ignore: &[String],
        on_event: &mut dyn FnMut(&str),
    ) -> Result<(), ClientError> {
        let req = Request::Sysupgrade {
            ignore: ignore.to_vec(),
        };
        self.request(&req, on_event)
    }

    /// `pacman -U`: `specs` are `validate::spec` lines (sha256 + abs path).
    pub(crate) fn install_files(
        &mut self,
        opts: FileOpts,
        specs: &[String],
    ) -> Result<(), ClientError> {
        let req = Request::InstallFiles {
            opts,
            files: specs.to_vec(),
        };
        self.request(&req, &mut |_| {})
    }

    /// `pacman -D --asexplicit` (true) / `--asdeps` (false).
    pub(crate) fn set_reason(
        &mut self,
        explicit: bool,
        names: &[String],
    ) -> Result<(), ClientError> {
        let req = Request::SetReason {
            explicit,
            names: names.to_vec(),
        };
        self.request(&req, &mut |_| {})
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !self.broken {
            let _ = proto::write_request(&mut self.tx, &Request::Quit);
        }
        // Dropping the real writer closes the pipe: EOF tells the helper to stop.
        self.tx = Box::new(io::sink());
        if let Some(mut c) = self.child.take() {
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::serve::{self, Backend};
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread::{self, JoinHandle};

    #[derive(Default)]
    struct Rec {
        lines: Vec<(Target, String)>,
        fail: bool,
    }

    impl Backend for Rec {
        fn append(&mut self, target: Target, line: &str) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::new(io::ErrorKind::Other, "boom"));
            }
            self.lines.push((target, line.to_string()));
            Ok(())
        }
    }

    fn client_on(b: UnixStream) -> Client {
        Client::from_streams(
            Box::new(BufReader::new(b.try_clone().unwrap())),
            Box::new(b),
            None,
        )
    }

    /// Client wired to a real `serve` loop in a thread.
    fn pair(be: Rec) -> (Client, JoinHandle<Rec>) {
        let (a, b) = UnixStream::pair().unwrap();
        let h = thread::spawn(move || {
            let mut r = BufReader::new(a.try_clone().unwrap());
            let mut w = a;
            let mut be = be;
            let _ = serve::serve(&mut r, &mut w, &mut be);
            be
        });
        (client_on(b), h)
    }

    #[test]
    fn ping_and_append_roundtrip() {
        let (mut c, h) = pair(Rec::default());
        c.ping().unwrap();
        c.append(Target::EmergeLog, "hello").unwrap();
        drop(c); // sends quit, server loop ends
        let be = h.join().unwrap();
        assert_eq!(be.lines, vec![(Target::EmergeLog, "hello".to_string())]);
    }

    #[test]
    fn refusal_is_an_error_but_not_fatal() {
        let (mut c, h) = pair(Rec {
            fail: true,
            ..Default::default()
        });
        match c.append(Target::World, "x") {
            Err(ClientError::Refused(m)) => assert_eq!(m, "boom"),
            other => panic!("unexpected: {:?}", other),
        }
        c.ping().unwrap();
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn bad_value_is_refused_locally_and_stream_stays_usable() {
        let (mut c, h) = pair(Rec::default());
        assert!(matches!(
            c.append(Target::World, "a\nCMD sync"),
            Err(ClientError::Proto(_))
        ));
        c.ping().unwrap();
        drop(c);
        let be = h.join().unwrap();
        assert!(be.lines.is_empty());
    }

    #[test]
    fn dead_helper_poisons_the_client() {
        let (a, b) = UnixStream::pair().unwrap();
        drop(a);
        let mut c = client_on(b);
        assert!(matches!(c.ping(), Err(ClientError::Proto(_))));
        assert!(matches!(c.ping(), Err(ClientError::Broken)));
    }

    #[test]
    fn events_arrive_before_the_final_ok() {
        let (a, b) = UnixStream::pair().unwrap();
        let h = thread::spawn(move || {
            let mut r = BufReader::new(a.try_clone().unwrap());
            let mut w = a;
            let _ = proto::read_request(&mut r).unwrap();
            proto::write_response(&mut w, &Response::Event("1/2".into())).unwrap();
            proto::write_response(&mut w, &Response::Event("2/2".into())).unwrap();
            proto::write_response(&mut w, &Response::Done).unwrap();
        });
        let mut c = client_on(b);
        let mut seen = Vec::new();
        c.request(&Request::Sync { force: false }, &mut |t| {
            seen.push(t.to_string())
        })
        .unwrap();
        assert_eq!(seen, vec!["1/2", "2/2"]);
        h.join().unwrap();
    }

    #[test]
    fn helper_that_exits_at_once_is_no_helper() {
        let r = Client::spawn("/bin/sh", &["-c", "exit 0"]);
        assert!(matches!(r, Err(ClientError::NoHelper)));
    }

    #[test]
    fn missing_program_is_a_spawn_error() {
        let r = Client::spawn("/nonexistent/sudo", &[]);
        assert!(matches!(r, Err(ClientError::Spawn(_))));
    }
}
