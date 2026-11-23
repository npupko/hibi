//! One provider child process and its three pipes (section 13.1).
//!
//! Nothing here blocks without a limit, whatever the provider does:
//!
//! - Standard input is written by a thread, so that a provider that stops
//!   reading cannot block the caller. The caller hands over a line and then
//!   waits for the response with a deadline that covers both.
//! - Standard output is read by a thread that splits it into lines of at
//!   most a given length and hands them over a rendezvous channel: the
//!   thread holds one finished line until a request takes it, and reads no
//!   further. A provider that writes when nothing was asked stalls on a
//!   full pipe; nothing accumulates in this process.
//! - Standard error is drained by a thread that keeps the last
//!   [`STDERR_TAIL_BYTES`] bytes.
//! - While a response is awaited the exit status of the child is polled, so
//!   an exit is noticed when it happens, also when another process (one the
//!   provider started) keeps the pipes open.
//!
//! The three threads end when their pipe closes. After the provider is
//! killed that is at once, unless a process the provider started still
//! holds the pipe: then the thread stays blocked in a read or a write until
//! that process closes it. Such a thread holds no lock and at most one
//! line, and the caller never waits for it.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use wait_timeout::ChildExt;

/// How many bytes of the provider's standard error hibi keeps.
pub const STDERR_TAIL_BYTES: usize = 4096;

/// How many bytes of output hibi reads after the last response, while a
/// provider shuts down. More than this is a failure and is not stored.
pub const MAX_TRAILING_BYTES: usize = 65_536;

/// How often the exit status of the child is looked at while a line is
/// awaited.
const EXIT_POLL: Duration = Duration::from_millis(20);

/// How long output is still awaited after the child was seen to have
/// exited: what it wrote before exiting is in the pipe and arrives within
/// this time.
const EXIT_SETTLE: Duration = Duration::from_millis(200);

/// What the standard-output thread reports.
enum Event {
    /// One line, without its terminating U+000A.
    Line(Vec<u8>),
    /// A line grew past the limit. The thread stops reading.
    TooLong,
    /// End of output, or a read error. `unterminated` is true when bytes
    /// came after the last U+000A; they are not a line and are dropped.
    End {
        why: Option<String>,
        unterminated: bool,
    },
}

/// Why no line could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadError {
    Timeout,
    TooLong,
    /// The output ended: the process exited or closed standard output.
    /// `unterminated` is true when it ended with bytes that no U+000A
    /// followed.
    Ended {
        why: Option<String>,
        unterminated: bool,
    },
    /// The request could not be written.
    WriteFailed(String),
}

/// What [`Process::finish`] found after standard input was closed.
pub(crate) struct Finished {
    /// The lines written after the last response, at most
    /// [`MAX_TRAILING_BYTES`] in total.
    pub(crate) lines: Vec<Vec<u8>>,
    /// The exit status of the child, or `None` when it had to be killed.
    pub(crate) status: Option<ExitStatus>,
    /// A violation in the trailing output: too much of it, a line over the
    /// limit, or bytes that no U+000A followed.
    pub(crate) problem: Option<String>,
}

pub(crate) struct Process {
    child: Child,
    /// Lines for the writer thread. `None` once standard input is closed.
    writer: Option<Sender<Vec<u8>>>,
    write_errors: Receiver<io::Error>,
    lines: Receiver<Event>,
    ended: Option<ReadError>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    reaped: Option<ExitStatus>,
}

impl Process {
    /// Start `argv` without a shell: `argv[0]` is the program, the rest are
    /// its arguments, passed as they are.
    pub(crate) fn spawn(
        argv: &[OsString],
        cwd: &Path,
        env: &[(&str, &OsString)],
        max_line: usize,
    ) -> io::Result<Process> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the command is empty"))?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        // The writer: one line per message. It ends, and closes standard
        // input, when the sender is dropped or a write fails.
        let (writer, outgoing) = mpsc::channel::<Vec<u8>>();
        let (write_failed, write_errors) = mpsc::channel();
        thread::spawn(move || {
            for bytes in outgoing {
                if let Err(e) = stdin.write_all(&bytes).and_then(|()| stdin.flush()) {
                    let _ = write_failed.send(e);
                    return;
                }
            }
        });

        // The reader: capacity 0, so a finished line is handed over only
        // when a request waits for it or looks for it.
        let (tx, lines) = mpsc::sync_channel(0);
        thread::spawn(move || {
            let mut r = BufReader::with_capacity(64 * 1024, stdout);
            loop {
                let event = read_line(&mut r, max_line);
                let last = !matches!(event, Event::Line(_));
                if tx.send(event).is_err() || last {
                    return;
                }
            }
        });

        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)));
        let keep = Arc::clone(&tail);
        thread::spawn(move || {
            let mut stderr = stderr;
            let mut buf = [0u8; 8192];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    return;
                }
                let Ok(mut t) = keep.lock() else { return };
                t.extend(&buf[..n]);
                let over = t.len().saturating_sub(STDERR_TAIL_BYTES);
                t.drain(..over);
            }
        });

        Ok(Process {
            child,
            writer: Some(writer),
            write_errors,
            lines,
            ended: None,
            stderr: tail,
            reaped: None,
        })
    }

    /// Hand one line and a U+000A to the writer. This never blocks: the
    /// time the write takes counts against the timeout of the
    /// [`Process::read_line`] that follows.
    ///
    /// A line of output that is already waiting was written when no request
    /// was outstanding. That is a protocol violation and an error here.
    pub(crate) fn send(&mut self, line: &str) -> io::Result<()> {
        match self.lines.try_recv() {
            Ok(Event::Line(_)) => {
                return Err(io::Error::other(
                    "the provider wrote output when no request was outstanding",
                ));
            }
            Ok(event) => self.ended = Some(end_of(event)),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.ended.get_or_insert(ReadError::Ended {
                    why: None,
                    unterminated: false,
                });
            }
        }
        if let Ok(e) = self.write_errors.try_recv() {
            return Err(e);
        }
        let closed = || io::Error::new(io::ErrorKind::BrokenPipe, "standard input is closed");
        let writer = self.writer.as_ref().ok_or_else(closed)?;
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        writer.send(bytes).map_err(|_| closed())
    }

    /// The exit status of the child when it has exited, without waiting.
    fn exited(&mut self) -> Option<ExitStatus> {
        if self.reaped.is_none() {
            self.reaped = self.child.try_wait().ok().flatten();
        }
        self.reaped
    }

    /// The next line of standard output, waiting at most `timeout`. The
    /// wait ends early when the child exits without having written one, or
    /// when the pending request could not be written.
    pub(crate) fn read_line(&mut self, timeout: Duration) -> Result<Vec<u8>, ReadError> {
        if let Some(e) = &self.ended {
            return Err(e.clone());
        }
        let deadline = Instant::now() + timeout;
        // Set when the child was seen to have exited.
        let mut settle_until: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let limit = settle_until.map_or(deadline, |s| s.min(deadline));
            if now >= limit {
                if settle_until.is_none() {
                    return Err(ReadError::Timeout);
                }
                let e = ReadError::Ended {
                    why: None,
                    unterminated: false,
                };
                self.ended = Some(e.clone());
                return Err(e);
            }
            match self.lines.recv_timeout((limit - now).min(EXIT_POLL)) {
                Ok(Event::Line(l)) => return Ok(l),
                Ok(event) => {
                    let e = end_of(event);
                    self.ended = Some(e.clone());
                    return Err(e);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let e = ReadError::Ended {
                        why: None,
                        unterminated: false,
                    };
                    self.ended = Some(e.clone());
                    return Err(e);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if settle_until.is_none() && self.exited().is_some() {
                        settle_until = Some(Instant::now() + EXIT_SETTLE);
                    } else if settle_until.is_none()
                        && let Ok(e) = self.write_errors.try_recv()
                    {
                        let e = ReadError::WriteFailed(e.to_string());
                        self.ended = Some(e.clone());
                        return Err(e);
                    }
                }
            }
        }
    }

    /// The last bytes the process wrote to standard error, as text.
    pub(crate) fn stderr_tail(&self) -> String {
        match self.stderr.lock() {
            Ok(t) => {
                let bytes: Vec<u8> = t.iter().copied().collect();
                String::from_utf8_lossy(&bytes).trim().to_owned()
            }
            Err(_) => String::new(),
        }
    }

    /// Kill the process and reap it. A write that the provider never read
    /// ends with the pipe.
    pub(crate) fn kill(&mut self) {
        self.writer = None;
        if self.reaped.is_none() {
            let _ = self.child.kill();
            self.reaped = self.child.wait().ok();
        }
    }

    /// Close standard input and wait up to `grace` for the process to exit.
    /// Returns its status, or `None` when it had to be killed. Output it
    /// writes meanwhile is not read: more than a pipe holds makes it stall
    /// and be killed at the end of `grace`.
    pub(crate) fn close(&mut self, grace: Duration) -> Option<ExitStatus> {
        self.writer = None;
        if let Some(status) = self.reaped {
            return Some(status);
        }
        match self.child.wait_timeout(grace) {
            Ok(Some(status)) => {
                self.reaped = Some(status);
                Some(status)
            }
            _ => {
                self.kill();
                None
            }
        }
    }

    /// Close standard input, then read what the process still writes until
    /// it exits or `grace` is over. At most [`MAX_TRAILING_BYTES`] are
    /// read; more is a problem and the process is killed.
    ///
    /// The verdict follows the exit of the child itself. The end of the
    /// output is not awaited beyond a short time after that exit, because a
    /// process the child started may keep the pipe open.
    pub(crate) fn finish(&mut self, grace: Duration) -> Finished {
        self.writer = None;
        let deadline = Instant::now() + grace;
        let mut lines = Vec::new();
        let mut total = 0usize;
        let mut problem = None;
        let mut settle_until: Option<Instant> = None;
        let mut output_ended = self.ended.is_some();
        loop {
            let now = Instant::now();
            if settle_until.is_none() && self.exited().is_some() {
                settle_until = Some(now + EXIT_SETTLE);
            }
            if let Some(status) = self.reaped
                && (output_ended || settle_until.is_some_and(|s| now >= s))
            {
                return Finished {
                    lines,
                    status: Some(status),
                    problem,
                };
            }
            if now >= deadline {
                self.kill();
                return Finished {
                    lines,
                    status: None,
                    problem,
                };
            }
            if output_ended {
                thread::sleep((deadline - now).min(EXIT_POLL));
                continue;
            }
            match self.lines.recv_timeout((deadline - now).min(EXIT_POLL)) {
                Ok(Event::Line(l)) => {
                    total += l.len() + 1;
                    if total > MAX_TRAILING_BYTES {
                        self.kill();
                        return Finished {
                            lines,
                            status: None,
                            problem: Some(format!(
                                "more than {MAX_TRAILING_BYTES} bytes of output after the last response"
                            )),
                        };
                    }
                    lines.push(l);
                }
                Ok(Event::TooLong) => {
                    self.kill();
                    return Finished {
                        lines,
                        status: None,
                        problem: Some("an output line over the length limit".to_owned()),
                    };
                }
                Ok(Event::End { unterminated, .. }) => {
                    output_ended = true;
                    if unterminated {
                        problem = Some(
                            "the output does not end with a line feed: its last bytes are not a line"
                                .to_owned(),
                        );
                    }
                }
                Err(RecvTimeoutError::Disconnected) => output_ended = true,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// The error for an event that ends the output.
fn end_of(event: Event) -> ReadError {
    match event {
        Event::TooLong => ReadError::TooLong,
        Event::End { why, unterminated } => ReadError::Ended { why, unterminated },
        Event::Line(_) => unreachable!("a line does not end the output"),
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Read one line of at most `max` bytes, not counting its U+000A.
fn read_line(r: &mut impl BufRead, max: usize) -> Event {
    let mut line = Vec::new();
    loop {
        let buf = match r.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Event::End {
                    why: Some(e.to_string()),
                    unterminated: !line.is_empty(),
                };
            }
        };
        if buf.is_empty() {
            return Event::End {
                why: None,
                unterminated: !line.is_empty(),
            };
        }
        let (take, done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (i, true),
            None => (buf.len(), false),
        };
        if line.len() + take > max {
            return Event::TooLong;
        }
        line.extend_from_slice(&buf[..take]);
        r.consume(if done { take + 1 } else { take });
        if done {
            return Event::Line(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_splitting_and_limit() {
        let mut r = io::Cursor::new(b"ab\n\ncdef\ntail".to_vec());
        assert!(matches!(read_line(&mut r, 4), Event::Line(l) if l == b"ab"));
        assert!(matches!(read_line(&mut r, 4), Event::Line(l) if l.is_empty()));
        assert!(matches!(read_line(&mut r, 4), Event::Line(l) if l == b"cdef"));
        // An unterminated rest is not a line, and the end says it was there.
        assert!(matches!(
            read_line(&mut r, 4),
            Event::End {
                why: None,
                unterminated: true
            }
        ));
        let mut r = io::Cursor::new(b"ab\n".to_vec());
        assert!(matches!(read_line(&mut r, 4), Event::Line(_)));
        assert!(matches!(
            read_line(&mut r, 4),
            Event::End {
                why: None,
                unterminated: false
            }
        ));
        let mut r = io::Cursor::new(b"abcde\n".to_vec());
        assert!(matches!(read_line(&mut r, 4), Event::TooLong));
    }
}
