// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A minimal client of the control port of tor (control-spec.txt): the
//! text protocol over TCP on 127.0.0.1, authenticated with the cookie.
//!
//! The app asks and tor answers, one command at a time; no events are
//! subscribed, so every line that comes is part of the reply to the last
//! command. A reply is lines of `<code><sep><text>`: `-` goes on, `+` opens
//! a data block that ends with a lone `.`, a space ends the reply.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

/// How long one command may wait for its reply.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// A reply line longer than this is no reply of tor.
const MAX_LINE: usize = 64 * 1024;

/// One line of a reply; a data block is its `data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyLine {
    pub code: u16,
    pub text: String,
    /// The lines of a `+` block, the dot-stuffing undone.
    pub data: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The code of the last line, which says how the command went.
    pub code: u16,
    pub lines: Vec<ReplyLine>,
}

impl Reply {
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// The text of the last line: `OK`, or what went wrong.
    pub fn message(&self) -> &str {
        self.lines.last().map_or("", |l| l.text.as_str())
    }

    /// The value of `key` in a reply to GETINFO: `key=value` on one line,
    /// or `key=` and a data block.
    pub fn value(&self, key: &str) -> Option<String> {
        self.lines.iter().find_map(|line| {
            let rest = line.text.strip_prefix(key)?.strip_prefix('=')?;
            Some(match &line.data {
                Some(data) => data.join("\n"),
                None => rest.to_string(),
            })
        })
    }
}

/// Builds replies from the lines that come, one at a time.
#[derive(Debug, Default)]
pub struct ReplyParser {
    lines: Vec<ReplyLine>,
    /// The line whose data block is being read.
    open: Option<ReplyLine>,
}

impl ReplyParser {
    /// Takes one line without its line end; returns the reply it completes.
    pub fn feed(&mut self, raw: &str) -> Result<Option<Reply>, String> {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(open) = &mut self.open {
            if raw == "." {
                let done = self.open.take().expect("a block is open");
                self.lines.push(done);
            } else {
                let line = raw.strip_prefix('.').map_or(raw, |rest| {
                    // `..` stands for a line that starts with a dot.
                    if raw.starts_with("..") {
                        rest
                    } else {
                        raw
                    }
                });
                open.data.get_or_insert_with(Vec::new).push(line.to_string());
            }
            return Ok(None);
        }
        let bytes = raw.as_bytes();
        if bytes.len() < 4 || !bytes[..3].iter().all(u8::is_ascii_digit) {
            return Err(format!("bad reply line from tor: {raw:?}"));
        }
        let code: u16 = raw[..3].parse().map_err(|_| format!("bad reply code: {raw:?}"))?;
        let text = raw[4..].to_string();
        match bytes[3] {
            b'-' => self.lines.push(ReplyLine { code, text, data: None }),
            b'+' => {
                self.open = Some(ReplyLine {
                    code,
                    text,
                    data: Some(Vec::new()),
                })
            }
            b' ' => {
                self.lines.push(ReplyLine { code, text, data: None });
                return Ok(Some(Reply {
                    code,
                    lines: std::mem::take(&mut self.lines),
                }));
            }
            _ => return Err(format!("bad reply line from tor: {raw:?}")),
        }
        Ok(None)
    }
}

/// Parses all of `text` as one reply.
pub fn parse_reply(text: &str) -> Result<Reply, String> {
    let mut parser = ReplyParser::default();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(reply) = parser.feed(line)? {
            return Ok(reply);
        }
    }
    Err("the reply of tor ended early".into())
}

/// `status/bootstrap-phase`: `NOTICE BOOTSTRAP PROGRESS=100 TAG=done
/// SUMMARY="Done"`, with `WARNING=… REASON=…` when it is stuck.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bootstrap {
    pub progress: u8,
    pub tag: String,
    pub summary: String,
    pub warning: Option<String>,
}

pub fn parse_bootstrap(value: &str) -> Bootstrap {
    let mut out = Bootstrap::default();
    for (key, value) in key_values(value) {
        match key.as_str() {
            "PROGRESS" => out.progress = value.parse::<u8>().unwrap_or(0).min(100),
            "TAG" => out.tag = value,
            "SUMMARY" => out.summary = value,
            "WARNING" => out.warning = Some(value),
            _ => {}
        }
    }
    out
}

/// The `KEY=value` and `KEY="quoted value"` pairs of a line; words without
/// `=` are skipped.
fn key_values(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ' ' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            continue;
        }
        chars.next();
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            value.push(next);
                        }
                    }
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ' ' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        out.push((key, value));
    }
    out
}

/// The ports of `net/listeners/socks`: `"127.0.0.1:9050" "[::1]:9050"`.
pub fn parse_listener_ports(value: &str) -> Vec<u16> {
    value
        .split_whitespace()
        .filter_map(|addr| {
            let addr = addr.trim_matches('"');
            addr.rsplit_once(':')?.1.parse().ok()
        })
        .collect()
}

/// The port of the file `ControlPortWriteToFile` writes: `PORT=127.0.0.1:9051`.
pub fn parse_port_file(text: &str) -> Option<u16> {
    text.lines().find_map(|line| {
        let addr = line.trim().strip_prefix("PORT=")?;
        addr.rsplit_once(':')?.1.parse().ok()
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// An authenticated control connection.
pub struct Control {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Control {
    /// Connects to the port in `port_file` and authenticates with the cookie
    /// in `cookie_file`.
    pub async fn connect(port_file: &Path, cookie_file: &Path) -> Result<Self, String> {
        let text = tokio::fs::read_to_string(port_file)
            .await
            .map_err(|e| format!("cannot read {}: {e}", port_file.display()))?;
        let port = parse_port_file(&text)
            .ok_or_else(|| format!("no port in {}", port_file.display()))?;
        let cookie = tokio::fs::read(cookie_file)
            .await
            .map_err(|e| format!("cannot read the control cookie: {e}"))?;
        let stream = tokio::time::timeout(
            Duration::from_secs(10),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        .map_err(|_| "the control port does not answer".to_string())?
        .map_err(|e| format!("cannot connect to the control port: {e}"))?;
        let (read, writer) = stream.into_split();
        let mut control = Self {
            reader: BufReader::new(read),
            writer,
        };
        control
            .expect_ok(&format!("AUTHENTICATE {}", hex(&cookie)))
            .await?;
        Ok(control)
    }

    /// Sends one command line and reads its reply.
    pub async fn command(&mut self, line: &str) -> Result<Reply, String> {
        if line.contains(['\r', '\n']) {
            return Err("a control command holds a line break".into());
        }
        self.writer
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|e| format!("control port: {e}"))?;
        tokio::time::timeout(REPLY_TIMEOUT, self.read_reply())
            .await
            .map_err(|_| "control port: no reply".to_string())?
    }

    async fn read_reply(&mut self) -> Result<Reply, String> {
        let mut parser = ReplyParser::default();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = (&mut self.reader)
                .take(MAX_LINE as u64)
                .read_until(b'\n', &mut buf)
                .await
                .map_err(|e| format!("control port: {e}"))?;
            if n == 0 {
                return Err("control port: closed".into());
            }
            if buf.last() != Some(&b'\n') {
                return Err("control port: a reply line too long".into());
            }
            buf.pop();
            let line = String::from_utf8_lossy(&buf);
            if let Some(reply) = parser.feed(&line)? {
                return Ok(reply);
            }
        }
    }

    /// A command whose reply must be 250.
    pub async fn expect_ok(&mut self, line: &str) -> Result<Reply, String> {
        let reply = self.command(line).await?;
        if reply.is_ok() {
            Ok(reply)
        } else {
            // The line itself may hold the cookie: only its verb is told.
            let verb = line.split_whitespace().next().unwrap_or("");
            Err(format!("{verb}: {} {}", reply.code, reply.message()))
        }
    }

    pub async fn get_info(&mut self, key: &str) -> Result<String, String> {
        let reply = self.expect_ok(&format!("GETINFO {key}")).await?;
        reply
            .value(key)
            .ok_or_else(|| format!("GETINFO {key}: no value in the reply"))
    }

    /// tor exits when this connection closes.
    pub async fn take_ownership(&mut self) -> Result<(), String> {
        self.expect_ok("TAKEOWNERSHIP").await.map(drop)
    }

    pub async fn bootstrap(&mut self) -> Result<Bootstrap, String> {
        Ok(parse_bootstrap(&self.get_info("status/bootstrap-phase").await?))
    }

    pub async fn socks_ports(&mut self) -> Result<Vec<u16>, String> {
        Ok(parse_listener_ports(&self.get_info("net/listeners/socks").await?))
    }

    pub async fn signal(&mut self, name: &str) -> Result<(), String> {
        self.expect_ok(&format!("SIGNAL {name}")).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_line_reply() {
        let reply = parse_reply("250 OK\r\n").unwrap();
        assert_eq!(reply.code, 250);
        assert!(reply.is_ok());
        assert_eq!(reply.message(), "OK");
    }

    #[test]
    fn error_replies() {
        let reply = parse_reply("515 Authentication failed: Wrong length on authentication cookie.\r\n").unwrap();
        assert_eq!(reply.code, 515);
        assert!(!reply.is_ok());
        assert!(reply.message().starts_with("Authentication failed"));
        let reply = parse_reply("552 Unrecognized key \"nope\"\r\n").unwrap();
        assert_eq!(reply.code, 552);
        assert_eq!(reply.value("nope"), None);
    }

    #[test]
    fn a_reply_of_many_lines_and_a_data_block() {
        let text = "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\"\r\n\
250+config-text=\r\n\
SocksPort auto\r\n\
..starts with a dot\r\n\
.\r\n\
250-net/listeners/socks=\"127.0.0.1:41234\" \"127.0.0.1:9150\"\r\n\
250 OK\r\n";
        let reply = parse_reply(text).unwrap();
        assert_eq!(reply.code, 250);
        assert_eq!(reply.lines.len(), 4);
        assert_eq!(
            reply.value("config-text").as_deref(),
            Some("SocksPort auto\n.starts with a dot")
        );
        assert_eq!(
            parse_listener_ports(&reply.value("net/listeners/socks").unwrap()),
            vec![41234, 9150]
        );
        let bootstrap = parse_bootstrap(&reply.value("status/bootstrap-phase").unwrap());
        assert_eq!(
            bootstrap,
            Bootstrap {
                progress: 100,
                tag: "done".into(),
                summary: "Done".into(),
                warning: None
            }
        );
    }

    #[test]
    fn the_parser_takes_line_by_line() {
        let mut parser = ReplyParser::default();
        assert_eq!(parser.feed("250-a=1").unwrap(), None);
        let reply = parser.feed("250 OK").unwrap().unwrap();
        assert_eq!(reply.value("a").as_deref(), Some("1"));
        // The next reply starts clean.
        let reply = parser.feed("250 OK").unwrap().unwrap();
        assert_eq!(reply.lines.len(), 1);
        assert!(parser.feed("garbage").is_err());
        assert!(parser.feed("25").is_err());
        assert!(parser.feed("250*x").is_err());
        assert!(parse_reply("250-a=1\r\n").is_err());
    }

    #[test]
    fn a_stuck_bootstrap_says_why() {
        let b = parse_bootstrap(
            r#"WARN BOOTSTRAP PROGRESS=10 TAG=conn_done SUMMARY="Connected to a relay" WARNING="Connection refused" REASON=CONNECTREFUSED COUNT=3 RECOMMENDATION=ignore HOSTID="ABC" HOSTADDR="1.2.3.4:443""#,
        );
        assert_eq!(b.progress, 10);
        assert_eq!(b.tag, "conn_done");
        assert_eq!(b.summary, "Connected to a relay");
        assert_eq!(b.warning.as_deref(), Some("Connection refused"));
        let b = parse_bootstrap(r#"NOTICE BOOTSTRAP PROGRESS=5 TAG=conn SUMMARY="Say \"hi\"""#);
        assert_eq!(b.summary, "Say \"hi\"");
        assert_eq!(parse_bootstrap("").progress, 0);
        assert_eq!(parse_bootstrap("PROGRESS=250").progress, 100);
        assert_eq!(parse_bootstrap("PROGRESS=x").progress, 0);
    }

    #[test]
    fn the_port_file() {
        assert_eq!(parse_port_file("PORT=127.0.0.1:38537\n"), Some(38537));
        assert_eq!(parse_port_file(""), None);
        assert_eq!(parse_listener_ports("\"[::1]:9050\""), vec![9050]);
        assert_eq!(hex(&[0, 0xab, 0x10]), "00AB10");
    }
}
