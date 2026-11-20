//! A scripted HTTP server on 127.0.0.1 that records every request and
//! answers from a script. One thread per connection; every response closes
//! its connection.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

/// One request as the server read it.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub raw: String,
    pub body: Value,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The question names of the request, sorted.
    pub fn names(&self) -> Vec<String> {
        self.body["questions"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Wait this long before answering.
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, body: &Value) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub fn status(status: u16) -> Reply {
        Reply::json(status, &json!({"detail": format!("scripted {status}")}))
    }

    pub fn header(mut self, k: &str, v: &str) -> Reply {
        self.headers.push((k.into(), v.into()));
        self
    }

    pub fn after(mut self, d: Duration) -> Reply {
        self.delay = d;
        self
    }
}

/// The script: the request and its number (0-based, in arrival order).
pub type Script = Arc<dyn Fn(&Recorded, usize) -> Reply + Send + Sync>;

pub struct Mock {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    /// TCP connections accepted.
    pub connections: Arc<AtomicUsize>,
    /// The most requests being handled at one time.
    pub max_in_flight: Arc<AtomicUsize>,
    script: Arc<Mutex<Script>>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl Mock {
    pub fn start(script: Script) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mock = Mock {
            url: format!("http://127.0.0.1:{port}/v1/systemone"),
            requests: Arc::default(),
            connections: Arc::default(),
            max_in_flight: Arc::default(),
            script: Arc::new(Mutex::new(script)),
            stop: Arc::default(),
            port,
        };
        let requests = mock.requests.clone();
        let connections = mock.connections.clone();
        let max = mock.max_in_flight.clone();
        let script = mock.script.clone();
        let stop = mock.stop.clone();
        let in_flight = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                connections.fetch_add(1, Ordering::SeqCst);
                let (requests, script, in_flight, max) = (
                    requests.clone(),
                    script.clone(),
                    in_flight.clone(),
                    max.clone(),
                );
                std::thread::spawn(move || {
                    let _ = handle(stream, &requests, &script, &in_flight, &max);
                });
            }
        });
        mock
    }

    /// A server that answers every question validly with `answer`.
    pub fn answering(
        answer: impl Fn(&str, &Value, &Value) -> Value + Send + Sync + 'static,
    ) -> Mock {
        Mock::start(Arc::new(move |r, _| {
            Reply::json(200, &answers(&r.body, &answer))
        }))
    }

    pub fn set_script(&self, script: Script) {
        *self.script.lock().unwrap() = script;
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Every request body as text, for "never sent" checks.
    pub fn all_bodies(&self) -> String {
        self.recorded()
            .iter()
            .map(|r| r.raw.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn handle(
    stream: TcpStream,
    requests: &Mutex<Vec<Recorded>>,
    script: &Mutex<Script>,
    in_flight: &AtomicUsize,
    max: &AtomicUsize,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    let mut headers = Vec::new();
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
            if k == "content-length" {
                length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf)?;
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let recorded = Recorded {
        method,
        path,
        headers,
        body: serde_json::from_str(&raw).unwrap_or(Value::Null),
        raw,
    };
    let n = {
        let mut r = requests.lock().unwrap();
        r.push(recorded.clone());
        r.len() - 1
    };
    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    max.fetch_max(now, Ordering::SeqCst);
    let script = script.lock().unwrap().clone();
    let reply = script(&recorded, n);
    std::thread::sleep(reply.delay);
    in_flight.fetch_sub(1, Ordering::SeqCst);
    let mut out = format!(
        "HTTP/1.1 {} Scripted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&reply.body);
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
    Ok(())
}

/// A valid response to `body`: for each question, `answer(name, question,
/// state)` gives either a number (a Noul probability, or for a Choice the
/// probability of the first option... see [`choice`]) or a full answer
/// object.
pub fn answers(body: &Value, answer: &impl Fn(&str, &Value, &Value) -> Value) -> Value {
    let mut out = Map::new();
    if let Some(qs) = body["questions"].as_object() {
        for (name, q) in qs {
            let a = answer(name, q, &body["state"]);
            let a = match (q["type"].as_str(), a) {
                (Some("noul"), Value::Number(n)) => json!({"type": "noul", "noul": n}),
                (_, a) => a,
            };
            out.insert(name.clone(), a);
        }
    }
    json!({"model": "jev-1.14.0", "answers": out, "usage": {"input_tokens": 10, "output_tokens": 1}})
}

/// A Choice answer that picks `choice` with probability `p` (the rest is
/// spread over the other options). `confidence` is deliberately the
/// opposite of `p`, so that a reader of `confidence` would be caught.
pub fn choice(question: &Value, choice: &str, p: f64) -> Value {
    let options: Vec<&String> = question["criteria"].as_object().unwrap().keys().collect();
    let rest = (1.0 - p) / (options.len() - 1) as f64;
    let probabilities: Map<String, Value> = options
        .iter()
        .map(|o| ((*o).clone(), json!(if *o == choice { p } else { rest })))
        .collect();
    json!({"type": "choice", "choice": choice, "confidence": 1.0 - p, "probabilities": probabilities})
}
