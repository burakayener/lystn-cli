//! Lystn server protocol — mirrors `cli/src/lystn/client.py::synthesize`.
//!
//! Flow (order matters — the WS must be connected & identified BEFORE the POST
//! so the server can route the audio back to this exact socket):
//!   1. Open WS `<ws>/stream`, send the first text frame `"<api_key|hello>|<session>"`.
//!   2. `POST <server>/speak` with JSON body + headers (see below).
//!   3. Read WS frames: binary = float32 PCM @ 24 kHz; text = JSON events.
//!      Stop on `{"event":"end"}` or `{"event":"error"}`.
//!
//! POST /speak headers (matching the Python client):
//!   - Content-Type: application/json   (set automatically by reqwest .json())
//!   - X-Lystn-Session: <session>       (unique per call)
//!   - X-Lystn-Key: <api_key>           (only if a key is configured)
//!   - X-Lystn-Terminal: <terminal>     (only if known — the agent session id)
//!   - X-Lystn-Source: <source>         (which assistant; default "claude")
//!
//! POST /speak body: {"text": ..., "voice"?: ..., "label"?: ...}

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio_tungstenite::tungstenite::Message;

use crate::audio::AudioPlayer;

#[derive(Serialize)]
struct SpeakBody<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
}

/// Parameters for one synthesize-and-play call.
pub struct SpeakParams<'a> {
    pub server: &'a str,
    pub api_key: Option<&'a str>,
    pub text: &'a str,
    pub voice: Option<&'a str>,
    pub label: Option<&'a str>,
    pub terminal: Option<&'a str>,
    pub source: &'a str,
    pub session: &'a str,
    pub speed: f64,
    pub volume: f64,
}

type DynErr = Box<dyn std::error::Error + Send + Sync>;

/// POST the text, then stream the returned PCM straight to the audio device.
pub async fn synthesize_and_play(p: SpeakParams<'_>) -> Result<(), DynErr> {
    let server = p.server.trim_end_matches('/');
    let ws_url = format!("{}/stream", http_to_ws(server));
    let speak_url = format!("{}/speak", server);

    // 1. Connect + identify the socket FIRST.
    let (ws_stream, _resp) = tokio_tungstenite::connect_async(&ws_url).await?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let ident = format!("{}|{}", p.api_key.unwrap_or("hello"), p.session);
    ws_tx.send(Message::Text(ident.into())).await?;

    // 2. POST /speak.
    let body = SpeakBody {
        text: p.text,
        voice: p.voice,
        label: p.label,
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut req = client
        .post(&speak_url)
        .header("X-Lystn-Session", p.session)
        .header("X-Lystn-Source", p.source)
        .json(&body);
    if let Some(key) = p.api_key {
        req = req.header("X-Lystn-Key", key);
    }
    if let Some(term) = p.terminal {
        req = req.header("X-Lystn-Terminal", term);
    }
    let resp = req.send().await?;
    let resp = resp.error_for_status()?;
    drop(resp);

    // 3. Stream PCM back and play it.
    let mut player = AudioPlayer::new(p.speed, p.volume).map_err(DynErr::from)?;
    while let Some(msg) = ws_rx.next().await {
        match msg? {
            Message::Binary(data) => {
                // float32 little-endian mono PCM @ 24 kHz.
                let mut samples = Vec::with_capacity(data.len() / 4);
                for frame in data.chunks_exact(4) {
                    let bytes = [frame[0], frame[1], frame[2], frame[3]];
                    samples.push(f32::from_le_bytes(bytes));
                }
                player.write(&samples);
            }
            Message::Text(text) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                    match v.get("event").and_then(|e| e.as_str()) {
                        // Persist the spoken line so the statusline can show it.
                        Some("speaking") => {
                            if let (Some(term), Some(spoken)) =
                                (p.terminal, v.get("text").and_then(|t| t.as_str()))
                            {
                                write_speaking(term, spoken);
                            }
                        }
                        Some("end") | Some("error") => break,
                        _ => {}
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    player.finish();
    let _ = ws_tx.close().await;
    Ok(())
}

/// http(s)://host -> ws(s)://host (mirrors `client.py::_http_to_ws`).
fn http_to_ws(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url.to_string()
    }
}

/// Persist the spoken text so the statusline can show it in the terminal,
/// keyed by the terminal/session id (which the statusline also receives).
fn write_speaking(terminal: &str, text: &str) {
    let path = std::env::temp_dir().join(format!("lystn-say-{terminal}.txt"));
    let _ = std::fs::write(path, text);
}
