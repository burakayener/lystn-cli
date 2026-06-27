//! Small config-mutating subcommands — `config`, `mute`/`unmute`, `speed`,
//! `volume`. These mirror the Python client (`cli/src/lystn/cli.py`) one-for-one
//! and never touch the network or audio device.

use serde_json::{json, Value};

use crate::config;

/// `lystn config show` — print the merged config (api_key masked) + the file
/// path, exactly like Python's `config_show`.
pub fn config_show() {
    let mut cfg = config::merged_map();
    if let Some(Value::String(k)) = cfg.get("api_key") {
        if !k.is_empty() {
            // Match Python: first 4 chars + "…(hidden)".
            let prefix: String = k.chars().take(4).collect();
            let masked = format!("{prefix}\u{2026}(hidden)");
            cfg.insert("api_key".into(), Value::String(masked));
        }
    }
    let text = serde_json::to_string_pretty(&Value::Object(cfg))
        .unwrap_or_else(|_| "{}".to_string());
    println!("{text}");
    println!("\nfile: {}", config::config_path().display());
}

/// `lystn config set <key> <value>` — update one key in config.json.
///
/// Supports the keys the task calls for: server, api_key, voice, speed, muted,
/// volume (plus device, which the Python client also accepts). Numeric/bool
/// values are coerced + clamped the same way the dedicated commands do.
pub fn config_set(key: &str, value: &str) {
    match key {
        "speed" => {
            let s: f64 = match value.parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("speed must be a number, e.g. 1.25");
                    std::process::exit(1);
                }
            };
            let clamped = s.clamp(0.5, 3.0);
            save("speed", json!(clamped));
            let note = if (clamped - s).abs() < f64::EPSILON {
                String::new()
            } else {
                format!(" (clamped from {})", py_float(s))
            };
            println!("saved speed: {}x{note}", py_float(clamped));
        }
        "volume" => {
            let v: f64 = match value.parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("volume must be a number 0-100");
                    std::process::exit(1);
                }
            };
            let clamped = (v as i64).clamp(0, 100);
            save("volume", json!(clamped));
            println!("saved volume: {clamped}%");
        }
        "muted" => {
            let b = parse_bool(value);
            save("muted", json!(b));
            println!("saved muted: {b}");
        }
        "device" => {
            if matches!(value.to_lowercase().as_str(), "default" | "none" | "") {
                save("device", Value::Null);
                println!("device: cleared (system default)");
            } else if let Ok(idx) = value.parse::<i64>() {
                save("device", json!(idx));
                println!("saved device: {idx}");
            } else {
                save("device", json!(value));
                println!("saved device: {value:?}");
            }
        }
        "api_key" | "server" | "voice" => {
            save(key, json!(value));
            println!("  [ok] saved {key}");
        }
        other => {
            eprintln!(
                "unknown key {other:?}; choose one of: server, api_key, voice, \
                 device, speed, muted, volume"
            );
            std::process::exit(1);
        }
    }
}

/// `lystn logout` — clear the saved API key from this machine (mirrors the
/// Python client setting api_key to None). The client falls back to anonymous
/// mode until the next `lystn login`.
pub fn logout() {
    save("api_key", json!(""));
    println!("  [ok] signed out — your API key was cleared from this machine");
}

/// `lystn mute` / `lystn unmute`.
pub fn set_muted(muted: bool) {
    save("muted", json!(muted));
    if muted {
        println!("muted - lystn will stay quiet. Run `lystn unmute` to turn it back on.");
    } else {
        println!("unmuted - lystn will speak again.");
    }
}

/// `lystn speed [value]` — show or set playback speed (clamped 0.5-3.0).
pub fn speed(value: Option<&str>) {
    let Some(value) = value else {
        let s = config::Config::load().clamped_speed();
        println!("{}x", py_float(s));
        return;
    };
    let s: f64 = match value.parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("speed must be a number, e.g. 1.5");
            std::process::exit(1);
        }
    };
    let clamped = s.clamp(0.5, 3.0);
    save("speed", json!(clamped));
    println!("speed set to {}x", py_float(clamped));
}

/// `lystn volume [value]` — show or set volume (0-100).
pub fn volume(value: Option<&str>) {
    let Some(value) = value else {
        let v = config::merged_map()
            .get("volume")
            .and_then(|x| x.as_f64())
            .unwrap_or(100.0) as i64;
        println!("{v}%");
        return;
    };
    let v: f64 = match value.parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("volume must be a number 0-100");
            std::process::exit(1);
        }
    };
    let clamped = (v as i64).clamp(0, 100);
    save("volume", json!(clamped));
    println!("volume set to {clamped}%");
}

/// Format a float the way Python's `str(float)` / f-strings do: an
/// integer-valued float keeps a trailing ".0" (e.g. 3.0 -> "3.0", 1.5 -> "1.5").
fn py_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

fn parse_bool(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn save(key: &str, value: Value) {
    if let Err(e) = config::set_value(key, value) {
        eprintln!("could not write config: {e}");
        std::process::exit(1);
    }
}
