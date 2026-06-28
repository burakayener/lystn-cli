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

// Standalone success/error pages shown in the browser after the CLI callback.
// Styled inline to match the website's dark brand palette
// (web/src/pages/index.astro :root vars). Byte strings are double-quote
// delimited, so all HTML/CSS quoting uses single quotes.
const PAGE_HEAD: &str = "<!doctype html><html lang=en><meta charset=utf-8>\
<meta name=viewport content='width=device-width,initial-scale=1'><title>Lystn</title>\
<link rel='preconnect' href='https://fonts.googleapis.com'>\
<link rel='preconnect' href='https://fonts.gstatic.com' crossorigin>\
<link href='https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap' rel='stylesheet'>\
<style>\
*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
padding:1.5rem;background:#08070d;color:#f5f3fc;line-height:1.5;\
font-family:'Inter',ui-sans-serif,system-ui,-apple-system,'Segoe UI',sans-serif;\
-webkit-font-smoothing:antialiased;\
background-image:radial-gradient(60rem 40rem at 50% -12%,rgba(109,94,252,.20),transparent 60%),\
radial-gradient(48rem 36rem at 92% 112%,rgba(39,224,196,.10),transparent 60%)}\
.card{text-align:center;max-width:25rem;width:100%;padding:2.75rem 2.25rem;\
border:1px solid rgba(255,255,255,.08);border-radius:20px;\
background:linear-gradient(180deg,#13121d,#0d0c15);\
box-shadow:0 40px 100px -30px rgba(0,0,0,.8),0 0 0 1px rgba(109,94,252,.06)}\
.brand{display:inline-flex;align-items:center;gap:9px;font-weight:800;font-size:19px;\
letter-spacing:-.02em;margin-bottom:1.6rem}\
.icon{width:58px;height:58px;border-radius:999px;margin:0 auto 1.2rem;display:flex;\
align-items:center;justify-content:center;font-size:1.7rem}\
.icon.ok{background:rgba(39,224,196,.12);color:#27e0c4;border:1px solid rgba(39,224,196,.3)}\
.icon.err{background:rgba(109,94,252,.12);color:#a99bff;border:1px solid rgba(109,94,252,.3)}\
h1{font-size:1.45rem;letter-spacing:-.025em;margin:0 0 .35rem;font-weight:800}\
p{color:#c8c4dc;margin:1.35rem 0 0;line-height:1.65;font-size:.95rem}\
code{font-family:'JetBrains Mono',ui-monospace,Menlo,monospace;color:#a99bff;\
background:rgba(109,94,252,.12);border:1px solid rgba(255,255,255,.08);\
border-radius:6px;padding:.1rem .42rem;font-size:.85em}\
.close{color:#8e8aa6;font-size:.8rem;margin-top:1.6rem}\
</style>";

// The Lystn ear wordmark glyph (single-quoted attrs for the byte-string context).
const BRAND_SVG: &str = "<svg width=28 height=28 viewBox='0 0 48 48' fill='none' aria-hidden='true'>\
<defs><linearGradient id='lg' x1='0' y1='0' x2='1' y2='1'>\
<stop offset='0' stop-color='#a99bff'/><stop offset='1' stop-color='#6d5efc'/></linearGradient></defs>\
<g stroke='url(#lg)' stroke-width='3.4' stroke-linecap='round' fill='none'>\
<path d='M30 22a8 8 0 1 0-3 6.4'/><path d='M25.5 22a3.5 3.5 0 1 0-1 2.5'/>\
<path d='M24 27.5c-1 2.4-3.2 4-5.6 3.6'/></g>\
<g stroke='#27e0c4' stroke-width='3.2' stroke-linecap='round' fill='none'>\
<path d='M33 18a8 8 0 0 1 0 12'/><path d='M37 14a13.5 13.5 0 0 1 0 20'/></g></svg>";

fn login_ok_html() -> String {
    format!(
        "{PAGE_HEAD}<div class=card><div class=brand>{BRAND_SVG}Lystn</div>\
<div class='icon ok'>&#10003;</div><h1>You're signed in to Lystn</h1>\
<p>Your key is saved. You can close this tab and return to the terminal.</p>\
</div></html>"
    )
}

fn login_bad_html() -> String {
    format!(
        "{PAGE_HEAD}<div class=card><div class=brand>{BRAND_SVG}Lystn</div>\
<div class='icon err'>!</div><h1>Sign-in could not be verified</h1>\
<p>Please run <code>lystn login</code> again.</p></div></html>"
    )
}

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

    let body = if ok { login_ok_html() } else { login_bad_html() };
    let _ = write_response(stream, "200 OK", body.as_bytes());

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
