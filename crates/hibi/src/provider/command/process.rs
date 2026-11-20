//! One provider child process and its three pipes (section 13.1).
//!
//! Standard output is read by a thread that splits it into lines of at most
//! a given length and hands them over a channel, so that a read can time
//! out. Standard error is drained by a second thread that keeps the last
//! [`STDERR_TAIL_BYTES`] bytes. Both threads end when the pipe closes.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use wait_timeout::ChildExt;

/// How many bytes of the provider's standard error hibi keeps.
pub const STDERR_TAIL_BYTES: usize = 4096;

/// What the standard-output thread reports.
enum Event {
    /// One line, without its terminating U+000A.
    Line(Vec<u8>),
    /// A line grew past the limit. The thread stops reading.
    TooLong,
    /// End of output, or a read error, with any unterminated rest dropped.
    End(Option<String>),
}

/// Why no line could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadError {
    Timeout,
    TooLong,
    /// The output ended: the process exited or closed standard output.
    Ended(Option<String>),
}

pub(crate) struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
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
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let (tx, lines) = mpsc::channel();
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
            stdin,
            lines,
            ended: None,
            stderr: tail,
            reaped: None,
        })
    }

    /// Write one line and a U+000A.
    pub(crate) fn send(&mut self, line: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "standard input is closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    /// The next line of standard output, waiting at most `timeout`.
    pub(crate) fn read_line(&mut self, timeout: Duration) -> Result<Vec<u8>, ReadError> {
        if let Some(e) = &self.ended {
            return Err(e.clone());
        }
        let e = match self.lines.recv_timeout(timeout) {
            Ok(Event::Line(l)) => return Ok(l),
            Err(RecvTimeoutError::Timeout) => return Err(ReadError::Timeout),
            Ok(Event::TooLong) => ReadError::TooLong,
            Ok(Event::End(why)) => ReadError::Ended(why),
            Err(RecvTimeoutError::Disconnected) => ReadError::Ended(None),
        };
        self.ended = Some(e.clone());
        Err(e)
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

    /// Kill the process and reap it.
    pub(crate) fn kill(&mut self) {
        self.stdin = None;
        if self.reaped.is_none() {
            let _ = self.child.kill();
            self.reaped = self.child.wait().ok();
        }
    }

    /// Close standard input and wait up to `grace` for the process to exit.
    /// Returns its status, or `None` when it had to be killed.
    pub(crate) fn close(&mut self, grace: Duration) -> Option<ExitStatus> {
        self.stdin = None;
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

    /// Close standard input, then read the remaining lines of standard
    /// output until it ends and wait for the exit, all within `grace`.
    /// Returns the lines and the exit status, or `None` when the process
    /// was killed at the deadline.
    pub(crate) fn finish(&mut self, grace: Duration) -> (Vec<Vec<u8>>, Option<ExitStatus>) {
        self.stdin = None;
        let deadline = Instant::now() + grace;
        let mut rest = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.read_line(left) {
                Ok(l) => rest.push(l),
                Err(ReadError::Ended(_)) => break,
                Err(_) => {
                    self.kill();
                    return (rest, None);
                }
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        (rest, self.close(left))
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
            Err(e) => return Event::End(Some(e.to_string())),
        };
        if buf.is_empty() {
            return Event::End(None);
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
        // An unterminated rest is dropped at the end of output.
        assert!(matches!(read_line(&mut r, 4), Event::End(None)));
        let mut r = io::Cursor::new(b"abcde\n".to_vec());
        assert!(matches!(read_line(&mut r, 4), Event::TooLong));
    }
}
