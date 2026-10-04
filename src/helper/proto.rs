//! Wire protocol, client <-> helper. Std only, no `crate::` imports.
//!
//! Text, line based, over inherited pipes.
//!   Request:  CMD <verb>\n  (ARG <value>\n)*  END\n
//!   Response: (EVT <text>\n)*  then  OK\n | ERR <text>\n
//! Every line is length-capped *before* it is parsed, and every value
//! goes through `validate` on both write and read.

use std::fmt;
use std::io::{self, BufRead, Write};

use super::validate::{self, Reject, Target, MAX_BATCH, MAX_LINE_LEN};

/// "ARG " / "ERR " prefix + longest legal value.
const MAX_WIRE_LINE: usize = MAX_LINE_LEN + 8;

#[derive(Debug)]
pub(crate) enum ProtoError {
    Io(io::Error),
    /// Peer closed mid-message.
    Eof,
    TooLong,
    BadUtf8,
    /// Stream is out of sync: nothing after this can be trusted.
    Framing(&'static str),
    /// Message was well-framed but its content is wrong.
    Syntax(&'static str),
    UnknownVerb,
    Invalid(Reject),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::Io(e) => write!(f, "io: {}", e),
            ProtoError::Eof => write!(f, "peer closed mid-message"),
            ProtoError::TooLong => write!(f, "line too long"),
            ProtoError::BadUtf8 => write!(f, "invalid utf-8"),
            ProtoError::Framing(m) => write!(f, "framing: {}", m),
            ProtoError::Syntax(m) => write!(f, "syntax: {}", m),
            ProtoError::UnknownVerb => write!(f, "unknown verb"),
            ProtoError::Invalid(r) => write!(f, "invalid value: {:?}", r),
        }
    }
}

impl ProtoError {
    /// Fatal = stop talking. Otherwise the next message is still in sync.
    pub(crate) fn is_fatal(&self) -> bool {
        matches!(
            self,
            ProtoError::Io(_)
                | ProtoError::Eof
                | ProtoError::TooLong
                | ProtoError::BadUtf8
                | ProtoError::Framing(_)
        )
    }
}

impl From<io::Error> for ProtoError {
    fn from(e: io::Error) -> Self {
        ProtoError::Io(e)
    }
}

impl From<Reject> for ProtoError {
    fn from(r: Reject) -> Self {
        ProtoError::Invalid(r)
    }
}

/// What the client may ask. Names only, no paths, no commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    Ping,
    Sync,
    Quit,
    Install(Vec<String>),
    Remove(Vec<String>),
    /// `asexplicit` / `asdeps`.
    SetReason {
        explicit: bool,
        names: Vec<String>,
    },
    /// One line appended to a whitelisted file.
    Append {
        target: Target,
        line: String,
    },
}

impl Request {
    fn to_wire(&self) -> (&'static str, Vec<String>) {
        match self {
            Request::Ping => ("ping", vec![]),
            Request::Sync => ("sync", vec![]),
            Request::Quit => ("quit", vec![]),
            Request::Install(n) => ("install", n.clone()),
            Request::Remove(n) => ("remove", n.clone()),
            Request::SetReason { explicit, names } => (
                if *explicit { "asexplicit" } else { "asdeps" },
                names.clone(),
            ),
            Request::Append { target, line } => {
                ("append", vec![target.id().to_string(), line.clone()])
            }
        }
    }

    /// The only place a verb + args become a `Request`; validates all.
    fn from_wire(verb: &str, args: Vec<String>) -> Result<Self, ProtoError> {
        let none = |r: Request, a: &[String]| {
            if a.is_empty() {
                Ok(r)
            } else {
                Err(ProtoError::Syntax("unexpected args"))
            }
        };
        match verb {
            "ping" => none(Request::Ping, &args),
            "sync" => none(Request::Sync, &args),
            "quit" => none(Request::Quit, &args),
            "install" => {
                validate::atoms(&args)?;
                Ok(Request::Install(args))
            }
            "remove" => {
                validate::atoms(&args)?;
                Ok(Request::Remove(args))
            }
            "asexplicit" | "asdeps" => {
                validate::atoms(&args)?;
                Ok(Request::SetReason {
                    explicit: verb == "asexplicit",
                    names: args,
                })
            }
            "append" => {
                let [t, l] = <[String; 2]>::try_from(args)
                    .map_err(|_| ProtoError::Syntax("append wants 2 args"))?;
                let target = Target::from_id(&t).ok_or(ProtoError::Syntax("unknown target"))?;
                validate::line(&l)?;
                Ok(Request::Append { target, line: l })
            }
            _ => Err(ProtoError::UnknownVerb),
        }
    }
}

/// Helper -> client. `Done`/`Fail` end a request, `Event` does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Response {
    Event(String),
    Done,
    Fail(String),
}

/// One line, hard-capped; `None` = clean EOF before any byte.
/// Never `read_line`: that would buffer an unbounded line.
fn read_line<R: BufRead>(r: &mut R) -> Result<Option<String>, ProtoError> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(ProtoError::Eof)
            };
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => {
                if buf.len() + i > MAX_WIRE_LINE {
                    return Err(ProtoError::TooLong);
                }
                buf.extend_from_slice(&chunk[..i]);
                r.consume(i + 1);
                break;
            }
            None => {
                let n = chunk.len();
                if buf.len() + n > MAX_WIRE_LINE {
                    return Err(ProtoError::TooLong);
                }
                buf.extend_from_slice(chunk);
                r.consume(n);
            }
        }
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| ProtoError::BadUtf8)
}

/// Helper side. `Ok(None)` = client hung up between requests.
pub(crate) fn read_request<R: BufRead>(r: &mut R) -> Result<Option<Request>, ProtoError> {
    let Some(first) = read_line(r)? else {
        return Ok(None);
    };
    let verb = first
        .strip_prefix("CMD ")
        .ok_or(ProtoError::Framing("expected CMD"))?;
    let mut args: Vec<String> = Vec::new();
    loop {
        let l = read_line(r)?.ok_or(ProtoError::Eof)?;
        if l == "END" {
            break;
        }
        let v = l
            .strip_prefix("ARG ")
            .ok_or(ProtoError::Framing("expected ARG or END"))?;
        if args.len() >= MAX_BATCH {
            return Err(ProtoError::Framing("too many args"));
        }
        args.push(v.to_string());
    }
    Request::from_wire(verb, args).map(Some)
}

/// Client side. Re-validates, so a bad value can't forge extra lines.
pub(crate) fn write_request<W: Write>(w: &mut W, req: &Request) -> Result<(), ProtoError> {
    let (verb, args) = req.to_wire();
    Request::from_wire(verb, args.clone())?;
    let mut out = format!("CMD {}\n", verb);
    for a in &args {
        out.push_str("ARG ");
        out.push_str(a);
        out.push('\n');
    }
    out.push_str("END\n");
    w.write_all(out.as_bytes())?;
    w.flush()?;
    Ok(())
}

/// Helper side.
pub(crate) fn write_response<W: Write>(w: &mut W, resp: &Response) -> Result<(), ProtoError> {
    let out = match resp {
        Response::Done => "OK\n".to_string(),
        Response::Event(t) => format!("EVT {}\n", validate::line(t)?),
        Response::Fail(t) => format!("ERR {}\n", validate::line(t)?),
    };
    w.write_all(out.as_bytes())?;
    w.flush()?;
    Ok(())
}

/// Client side. EOF here means the helper died: `Err(Eof)`.
pub(crate) fn read_response<R: BufRead>(r: &mut R) -> Result<Response, ProtoError> {
    let l = read_line(r)?.ok_or(ProtoError::Eof)?;
    if l == "OK" {
        return Ok(Response::Done);
    }
    if let Some(t) = l.strip_prefix("ERR ") {
        return Ok(Response::Fail(validate::line(t)?.to_string()));
    }
    if let Some(t) = l.strip_prefix("EVT ") {
        return Ok(Response::Event(validate::line(t)?.to_string()));
    }
    Err(ProtoError::Framing("bad response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(req: Request) {
        let mut buf = Vec::new();
        write_request(&mut buf, &req).unwrap();
        let got = read_request(&mut &buf[..]).unwrap().unwrap();
        assert_eq!(got, req);
    }

    fn parse(s: &str) -> Result<Option<Request>, ProtoError> {
        read_request(&mut s.as_bytes())
    }

    #[test]
    fn requests_roundtrip() {
        roundtrip(Request::Ping);
        roundtrip(Request::Sync);
        roundtrip(Request::Quit);
        roundtrip(Request::Install(vec!["extra/nano".into(), "vim".into()]));
        roundtrip(Request::Remove(vec!["old-pkg".into()]));
        roundtrip(Request::SetReason {
            explicit: true,
            names: vec!["foo".into()],
        });
        roundtrip(Request::SetReason {
            explicit: false,
            names: vec!["bar".into()],
        });
        roundtrip(Request::Append {
            target: Target::EmergeLog,
            line: "2026-09-21 09:10:05  UNMERGE  -      x-1-1".into(),
        });
    }

    #[test]
    fn two_requests_back_to_back_then_eof() {
        let data = "CMD ping\nEND\nCMD sync\nEND\n";
        let mut r = data.as_bytes();
        assert_eq!(read_request(&mut r).unwrap(), Some(Request::Ping));
        assert_eq!(read_request(&mut r).unwrap(), Some(Request::Sync));
        assert!(read_request(&mut r).unwrap().is_none());
    }

    #[test]
    fn truncated_message_is_eof_error() {
        assert!(matches!(
            parse("CMD install\nARG nano\n"),
            Err(ProtoError::Eof)
        ));
        assert!(matches!(parse("CMD ping"), Err(ProtoError::Eof)));
    }

    #[test]
    fn rejects_bad_framing() {
        assert!(matches!(parse("ping\nEND\n"), Err(ProtoError::Framing(_))));
        assert!(matches!(
            parse("CMD ping\nfoo\n"),
            Err(ProtoError::Framing(_))
        ));
        assert!(matches!(
            parse("CMD ping\nARG x\nEND\n"),
            Err(ProtoError::Syntax(_))
        ));
        assert!(matches!(
            parse("CMD rm-rf\nEND\n"),
            Err(ProtoError::UnknownVerb)
        ));
        assert!(matches!(
            parse("CMD ping\r\nEND\n"),
            Err(ProtoError::UnknownVerb)
        ));
        assert!(matches!(
            parse("CMD \u{0}\nEND\n"),
            Err(ProtoError::UnknownVerb)
        ));
    }

    #[test]
    fn rejects_bad_values() {
        assert!(matches!(
            parse("CMD install\nARG --noconfirm\nEND\n"),
            Err(ProtoError::Invalid(Reject::BadShape))
        ));
        assert!(matches!(
            parse("CMD install\nEND\n"),
            Err(ProtoError::Invalid(Reject::Empty))
        ));
        assert!(matches!(
            parse("CMD append\nARG /etc/shadow\nARG x\nEND\n"),
            Err(ProtoError::Syntax(_))
        ));
        assert!(matches!(
            parse("CMD append\nARG log\nEND\n"),
            Err(ProtoError::Syntax(_))
        ));
        assert!(matches!(
            parse("CMD append\nARG log\nARG a\u{1b}[2J\nEND\n"),
            Err(ProtoError::Invalid(Reject::BadChar))
        ));
    }

    #[test]
    fn line_and_batch_limits() {
        let long = format!("CMD {}\nEND\n", "a".repeat(MAX_WIRE_LINE + 1));
        assert!(matches!(parse(&long), Err(ProtoError::TooLong)));

        // No newline at all: must stop at the cap, not buffer forever.
        let endless = "a".repeat(MAX_WIRE_LINE * 4);
        assert!(matches!(parse(&endless), Err(ProtoError::TooLong)));

        let mut many = String::from("CMD install\n");
        for _ in 0..=MAX_BATCH {
            many.push_str("ARG a\n");
        }
        many.push_str("END\n");
        assert!(matches!(parse(&many), Err(ProtoError::Framing(_))));
    }

    #[test]
    fn fatal_vs_recoverable() {
        assert!(parse("junk\n").unwrap_err().is_fatal());
        assert!(parse("CMD ping\n").unwrap_err().is_fatal());
        assert!(!parse("CMD rm\nEND\n").unwrap_err().is_fatal());
        assert!(!parse("CMD install\nARG --x\nEND\n").unwrap_err().is_fatal());
        assert!(!parse("CMD ping\nARG x\nEND\n").unwrap_err().is_fatal());
    }

    #[test]
    fn bad_utf8_rejected() {
        let mut r: &[u8] = b"CMD \xff\nEND\n";
        assert!(matches!(read_request(&mut r), Err(ProtoError::BadUtf8)));
    }

    #[test]
    fn write_refuses_frame_forging() {
        let mut buf = Vec::new();
        let evil = Request::Install(vec!["nano\nEND\nCMD sync".into()]);
        assert!(write_request(&mut buf, &evil).is_err());
        assert!(buf.is_empty());
        let evil = Request::Append {
            target: Target::World,
            line: "a\nb".into(),
        };
        assert!(write_request(&mut buf, &evil).is_err());
        assert!(buf.is_empty());
    }

    #[test]
    fn responses_roundtrip() {
        let mut buf = Vec::new();
        write_response(&mut buf, &Response::Event("downloading 1/3".into())).unwrap();
        write_response(&mut buf, &Response::Fail("masked".into())).unwrap();
        write_response(&mut buf, &Response::Done).unwrap();
        let mut r = &buf[..];
        assert_eq!(
            read_response(&mut r).unwrap(),
            Response::Event("downloading 1/3".into())
        );
        assert_eq!(
            read_response(&mut r).unwrap(),
            Response::Fail("masked".into())
        );
        assert_eq!(read_response(&mut r).unwrap(), Response::Done);
        // helper died: EOF is an error, never a silent success
        assert!(matches!(read_response(&mut r), Err(ProtoError::Eof)));
    }

    #[test]
    fn response_text_is_validated() {
        let mut buf = Vec::new();
        assert!(write_response(&mut buf, &Response::Event("a\nOK".into())).is_err());
        assert!(buf.is_empty());
        assert!(matches!(
            read_response(&mut "ERR a\u{1b}[2J\n".as_bytes()),
            Err(ProtoError::Invalid(Reject::BadChar))
        ));
        assert!(matches!(
            read_response(&mut "WAT\n".as_bytes()),
            Err(ProtoError::Framing(_))
        ));
    }
}
