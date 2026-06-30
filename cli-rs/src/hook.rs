//! `lystn hook` — Stop-hook entry point for Claude Code / Codex.
//!
//! Reads the hook JSON from stdin as RAW BYTES (no locale-dependent decoding —
//! the old Python client's cp1254 mojibake bug is gone by construction), decodes
//! it as UTF-8, pulls out `last_assistant_message` (the reply to speak) and
//! `session_id` (the stable per-terminal id), then POSTs to `/speak` and plays
//! the streamed audio. Per the migration plan, the text is forwarded RAW — all
//! cleaning now lives on the VPS (CLEANER_1).
//!
//! It must NEVER propagate an error: a broken hook must not break Claude's UI.
//! Everything is logged to `<config_dir>/hook.log` and swallowed.

use std::io::Read;

use crate::api::{self, SpeakParams};
use crate::config::{self, Config};

/// Sync entry point (called from `main`). Builds a single-threaded Tokio runtime
/// and runs the async hook on it.
pub fn run(source_arg: Option<String>) {
    let source = source_arg
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("LYSTN_SOURCE").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "claude".to_string());

    config::hook_log(&format!("hook invoked (source={source})"));

    // Read stdin as raw bytes; decode UTF-8 lossily (matches the new flow).
    let mut bytes = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
        config::hook_log(&format!("could not read stdin: {e}"));
        return;
    }
    if bytes.is_empty() {
        config::hook_log("empty stdin, exiting");
        return;
    }
    let raw = String::from_utf8_lossy(&bytes);

    let payload: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            config::hook_log(&format!("bad JSON on stdin: {e}"));
            return;
        }
    };

    let raw_text = payload
        .get("last_assistant_message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if raw_text.trim().is_empty() {
        config::hook_log("no assistant message, skipping");
        return;
    }
    // PRIVACY: strip code blocks, file paths, URLs and source filenames on THIS
    // machine before anything is sent — the most sensitive content never leaves
    // the device. The server's CLEANER_1 still runs (idempotent) for cosmetics.
    let text = crate::clean::pre_clean(&raw_text);
    if text.trim().is_empty() {
        config::hook_log("nothing speakable after local clean, skipping");
        return;
    }

    let cfg = Config::load();
    if cfg.muted {
        config::hook_log("muted (lystn mute); skipping playback");
        return;
    }

    let terminal = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Per-terminal voice: env LYSTN_VOICE wins, else the saved config voice.
    let voice = std::env::var("LYSTN_VOICE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| cfg.voice.clone());
    // Per-terminal spoken label from env LYSTN_LABEL.
    let label = std::env::var("LYSTN_LABEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let speed = cfg.clamped_speed();
    let volume = cfg.volume_gain();
    let session = new_session_id();

    config::hook_log(&format!(
        "speaking {} chars via {} (voice={voice:?}, label={label:?}, terminal={terminal:?}, speed={speed}, source={source})",
        text.chars().count(),
        cfg.server,
    ));

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            config::hook_log(&format!("could not start runtime: {e}"));
            return;
        }
    };

    let result = rt.block_on(async {
        api::synthesize_and_play(SpeakParams {
            server: &cfg.server,
            api_key: cfg.api_key.as_deref(),
            text: &text,
            voice: Some(&voice),
            label: label.as_deref(),
            terminal: terminal.as_deref(),
            source: &source,
            session: &session,
            speed,
            volume,
        })
        .await
    });

    match result {
        Ok(()) => config::hook_log("playback complete"),
        // Swallow — Claude's UI must not break.
        Err(e) => config::hook_log(&format!("playback FAILED: {e}")),
    }
}

/// A unique-per-invocation session id (hex). Each hook is its own process, so
/// epoch-nanos + pid is unique enough; this avoids a uuid/rand dependency.
fn new_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{nanos:032x}{pid:08x}")
}
