//! User config — read from the SAME JSON file the Python client uses.
//!
//! Location (mirrors `cli/src/lystn/config.py::config_dir`):
//!   - Windows: `%APPDATA%\lystn\config.json`
//!     (fallback `~\AppData\Roaming\lystn\config.json`)
//!   - other:   `$XDG_CONFIG_HOME/lystn/config.json`
//!     (fallback `~/.config/lystn/config.json`)
//!
//! Keys (all optional, null-tolerant): server, api_key, voice, device, speed,
//! muted, volume. Defaults match the Python `DEFAULTS` dict.

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub server: String,
    pub api_key: Option<String>,
    pub voice: String,
    pub speed: f64,
    pub muted: bool,
    /// 0..=100 (percent), as stored by the Python client.
    pub volume: f64,
}

impl Default for Config {
    fn default() -> Self {
        // Mirrors cli/src/lystn/config.py DEFAULTS.
        Config {
            server: "https://api.lystn.space".to_string(),
            api_key: None,
            voice: "af_heart".to_string(),
            speed: 1.0,
            muted: false,
            volume: 100.0,
        }
    }
}

impl Config {
    /// Load + merge over defaults, ignoring null/empty values (same as Python).
    pub fn load() -> Config {
        let mut c = Config::default();
        let path = config_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return c, // no file yet -> defaults
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return c, // unparseable -> defaults (Python does the same)
        };
        if let Some(s) = v.get("server").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                c.server = s.to_string();
            }
        }
        if let Some(s) = v.get("api_key").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                c.api_key = Some(s.to_string());
            }
        }
        if let Some(s) = v.get("voice").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                c.voice = s.to_string();
            }
        }
        if let Some(n) = v.get("speed").and_then(|x| x.as_f64()) {
            c.speed = n;
        }
        if let Some(b) = v.get("muted").and_then(|x| x.as_bool()) {
            c.muted = b;
        }
        if let Some(n) = v.get("volume").and_then(|x| x.as_f64()) {
            c.volume = n;
        }
        c
    }

    /// Playback speed clamped to the Python client's 0.5..=3.0 range.
    pub fn clamped_speed(&self) -> f64 {
        self.speed.clamp(0.5, 3.0)
    }

    /// Volume as a 0.0..=1.0 gain (Python stores 0..100).
    pub fn volume_gain(&self) -> f64 {
        (self.volume / 100.0).clamp(0.0, 1.0)
    }
}

pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
    };
    base.join("lystn")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// The Python client's `DEFAULTS` dict, in insertion order, as a JSON map.
/// (Mirrors `cli/src/lystn/config.py::DEFAULTS`.)
pub fn defaults_map() -> serde_json::Map<String, serde_json::Value> {
    use serde_json::{json, Value};
    let mut m = serde_json::Map::new();
    m.insert("server".into(), json!("https://api.lystn.space"));
    m.insert("api_key".into(), Value::Null);
    m.insert("voice".into(), json!("af_heart"));
    m.insert("device".into(), Value::Null);
    m.insert("speed".into(), json!(1.0));
    m.insert("muted".into(), json!(false));
    m.insert("volume".into(), json!(100));
    m
}

/// Merged config = DEFAULTS updated with the file's non-null entries (mirrors
/// `cli/src/lystn/config.py::load`). Used by `config show` and `config set`.
pub fn merged_map() -> serde_json::Map<String, serde_json::Value> {
    let mut merged = defaults_map();
    let text = match fs::read_to_string(config_path()) {
        Ok(t) => t,
        Err(_) => return merged,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return merged, // unparseable -> defaults (Python does the same)
    };
    if let serde_json::Value::Object(data) = parsed {
        for (k, v) in data {
            if !v.is_null() {
                merged.insert(k, v);
            }
        }
    }
    merged
}

/// Mirror `config.py::set_value`: load the merged config, set one key, and write
/// the whole thing back with 2-space indent (no trailing newline, like
/// `json.dump`). Creates the config dir if needed.
pub fn set_value(key: &str, value: serde_json::Value) -> std::io::Result<()> {
    let mut cfg = merged_map();
    cfg.insert(key.to_string(), value);
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(cfg))
        .unwrap_or_else(|_| "{}".to_string());
    fs::write(config_path(), text)
}

/// Append a diagnostic line to `<config_dir>/hook.log`.
///
/// The Stop hook runs as a Claude Code subprocess, so its stdout is invisible —
/// a file log is the only way to find out why a hook silently failed. Mirrors
/// `cli/src/lystn/cli.py::_hook_log`. Never panics.
pub fn hook_log(msg: &str) {
    use std::io::Write;
    let dir = config_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("hook.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", now_iso(), msg);
    }
}

/// Best-effort local timestamp without pulling in a date crate. Seconds since
/// the Unix epoch is enough to correlate log lines.
fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => format!("t+{}s", d.as_secs()),
        Err(_) => "t+?".to_string(),
    }
}
