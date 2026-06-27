//! `lystn login` — browser sign-in that stores the API key automatically.
//!
//! Mirrors `cli/src/lystn/cli.py::login`: open a one-shot localhost listener on
//! an ephemeral port, send the browser to `<server>/login/google?port=&state=`,
//! and wait for the server's `GET /callback?state=&key=&email=`. On a matching
//! state + non-empty key we save the key (and the server we logged in against)
//! to config.json. A wrong-state hit shows the error page but keeps waiting (it
//! may be a stray/forged request) until the 300s timeout.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::config::{self, Config};
use crate::ui;

const LOGIN_OK_HTML: &[u8] = b"<!doctype html><meta charset=utf-8><title>Lystn</title>\
<body style='font-family:system-ui;max-width:32rem;margin:4rem auto;text-align:center'>\
<h1>You're signed in to Lystn</h1>\
<p>Your key is saved. You can close this tab and return to the terminal.</p></body>";

const LOGIN_BAD_HTML: &[u8] = b"<!doctype html><meta charset=utf-8><title>Lystn</title>\
<body style='font-family:system-ui;max-width:32rem;margin:4rem auto;text-align:center'>\
<h1>Lystn</h1><p>Sign-in could not be verified. Please run <code>lystn login</code> again.</p></body>";

pub fn run(server_override: Option<&str>) {
    let cfg = Config::load();
    let srv = server_override
        .map(|s| s.to_string())
        .unwrap_or(cfg.server)
        .trim_end_matches('/')
        .to_string();
    let state = random_token();

    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            ui::bad(&format!("could not start local listener: {e}"));
            std::process::exit(1);
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            ui::bad(&format!("could not read listener port: {e}"));
            std::process::exit(1);
        }
    };

    let (tx, rx) = mpsc::channel::<(String, String)>();
    let state_for_thread = state.clone();
    thread::spawn(move || serve(listener, &state_for_thread, tx));

    let url = format!("{srv}/login/google?port={port}&state={state}");
    ui::banner("login");
    println!("  Opening your browser to sign in with Google...");
    open_browser(&url);
    ui::grey(&format!("  If it didn't open, visit:\n  {url}"));

    match rx.recv_timeout(Duration::from_secs(300)) {
        Ok((key, email)) => {
            let _ = config::set_value("api_key", json!(key));
            // Persist the server we logged in against, so later commands match.
            let _ = config::set_value("server", json!(srv));
            let who = if email.is_empty() {
                "your account".to_string()
            } else {
                email
            };
            ui::ok(&format!("signed in as {who}"));
            ui::grey(
                "  Run `lystn test` to hear it, then `lystn install` to wire your assistant.",
            );
        }
        Err(_) => {
            ui::bad("login timed out — run `lystn login` again");
            std::process::exit(1);
        }
    }
}

/// Accept loop: serve until a valid `/callback` arrives, then send the key/email
/// on `tx` and stop. Stray/invalid hits get an error page but don't end the wait.
fn serve(listener: TcpListener, state: &str, tx: mpsc::Sender<(String, String)>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let Some((key, email)) = handle(stream, state) {
            let _ = tx.send((key, email));
            return;
        }
    }
}

/// Handle one connection. Returns Some((key, email)) only on a valid callback.
fn handle(mut stream: TcpStream, state: &str) -> Option<(String, String)> {
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return None;
    }
    // Drain the rest of the headers so the client sees a clean response.
    let mut line = String::new();
    while reader.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
        if line == "\r\n" || line == "\n" {
            break;
        }
        line.clear();
    }

    // "GET /callback?state=...&key=... HTTP/1.1"
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let (route, query) = match path.split_once('?') {
        Some((r, q)) => (r, q),
        None => (path, ""),
    };

    if route != "/callback" {
        let _ = write_response(stream, "404 Not Found", b"not found");
        return None;
    }

    let got_state = query_param(query, "state").unwrap_or_default();
    let key = query_param(query, "key").unwrap_or_default();
    let email = query_param(query, "email").unwrap_or_default();
    let ok = !key.is_empty() && got_state == state;

    let body = if ok { LOGIN_OK_HTML } else { LOGIN_BAD_HTML };
    let _ = write_response(stream, "200 OK", body);

    if ok {
        Some((key, email))
    } else {
        None
    }
}

fn write_response(mut stream: TcpStream, status: &str, body: &[u8]) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Pull one value out of a `a=1&b=2` query string, URL-decoded.
fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(url_decode(v));
        }
    }
    None
}

/// Minimal `application/x-www-form-urlencoded` decode (`+` -> space, `%XX`).
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 16 random bytes, hex-encoded (URL-safe). Falls back to a time/pid mix if the
/// OS RNG is unavailable, which is fine for this local-handshake nonce.
fn random_token() -> String {
    let mut buf = [0u8; 16];
    if getrandom::fill(&mut buf).is_err() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = nanos ^ ((std::process::id() as u128) << 64);
        buf.copy_from_slice(&mix.to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn open_browser(url: &str) {
    use std::process::Command;
    let result = if cfg!(target_os = "windows") {
        // NOT `cmd /C start`: cmd treats `&` in the URL as a command separator,
        // so it drops everything from `&state=...` on. The server then mints its
        // own state and the callback never matches ("Sign-in could not be
        // verified"). rundll32 takes the whole URL as one clean argument.
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    let _ = result; // best-effort; the URL is also printed for manual use.
}
