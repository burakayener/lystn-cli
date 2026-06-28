//! `lystn install` / `lystn uninstall` — wire (or unwire) Lystn into Claude Code
//! (`~/.claude/settings.json`, JSON) and Codex (`~/.codex/config.toml`, TOML).
//!
//! Mirrors `cli/src/lystn/cli.py`: a bare `install` wires BOTH assistants
//! UNCONDITIONALLY (the Codex config is created if absent, so it works whether
//! or not Codex is installed). Each merge is non-destructive, idempotent (old
//! Lystn entries are swept before re-adding so a moved binary path refreshes),
//! and writes a `.bak` backup before changing anything.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::config;
use crate::ui;

const STATUS_MESSAGE: &str = "lystn \u{1f50a} speaking";
const CODEX_SOURCE: &str = "codex";

/// Absolute path to the CURRENTLY running binary, forward-slashed.
///
/// Claude Code (and Codex on Windows) run hook commands through bash, which eats
/// backslashes; forward slashes work for both bash and Windows path resolution.
/// Falls back to a bare "lystn" if the exe path can't be resolved.
fn lystn_exe() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "lystn".to_string())
}

fn claude_settings_path() -> PathBuf {
    config::home_dir().join(".claude").join("settings.json")
}

/// Codex config: `$CODEX_HOME/config.toml`, else `~/.codex/config.toml`.
fn codex_config_path() -> PathBuf {
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        PathBuf::from(home).join("config.toml")
    } else {
        config::home_dir().join(".codex").join("config.toml")
    }
}

/// The Codex Stop-hook command string. On Windows the POSIX env-prefix form is
/// broken (cmd.exe reads `LYSTN_SOURCE=codex` as a program name), so use the
/// shell-agnostic `--source` flag; elsewhere keep the POSIX env-prefix form.
fn codex_hook_command() -> String {
    let exe = lystn_exe();
    if cfg!(windows) {
        format!("{exe} hook --source {CODEX_SOURCE}")
    } else {
        format!("LYSTN_SOURCE={CODEX_SOURCE} {exe} hook")
    }
}

/// Best-effort: is any local Codex surface present? (CODEX_HOME, a ~/.codex dir,
/// or a `codex` on PATH.) Only used to pick the wording — install runs anyway.
fn codex_detected() -> bool {
    if std::env::var_os("CODEX_HOME").is_some() {
        return true;
    }
    if config::home_dir().join(".codex").is_dir() {
        return true;
    }
    which_codex()
}

fn which_codex() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .map(|s| s.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let cand = dir.join(format!("codex{ext}"));
            if cand.is_file() {
                return true;
            }
        }
    }
    false
}

// --- "is this a Lystn entry?" predicates (mirror the Python helpers) ---------

fn is_lystn_command(cmd: &str) -> bool {
    let cmd = cmd.to_lowercase();
    let markers = ["spoken aloud by lystn", "spoken aloud by a tts engine"];
    if markers.iter().any(|m| cmd.contains(m)) {
        return true;
    }
    if !cmd.contains("lystn") {
        return false;
    }
    cmd.split_whitespace().any(|t| t == "hook")
}

fn is_lystn_entry(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(is_lystn_command)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn is_lystn_statusline(sl: &Value) -> bool {
    let Some(cmd) = sl.get("command").and_then(|c| c.as_str()) else {
        return false;
    };
    let cmd = cmd.to_lowercase();
    cmd.contains("lystn") && cmd.split_whitespace().any(|t| t == "statusline")
}

/// The Stop-hook block Lystn needs in Claude Code's settings.json.
fn lystn_hooks() -> Value {
    json!({
        "Stop": [
            {
                "matcher": "*",
                "hooks": [
                    {"type": "command", "command": format!("{} hook", lystn_exe()),
                     "statusMessage": STATUS_MESSAGE}
                ]
            }
        ]
    })
}

// --- Snippet printers --------------------------------------------------------

fn print_snippet() {
    println!("Add the following `hooks` key inside the top-level object");
    println!("of ~/.claude/settings.json (merge with existing keys, comma-separated):\n");
    let pretty = serde_json::to_string_pretty(&lystn_hooks()).unwrap_or_default();
    println!("\"hooks\": {pretty}");
}

fn print_codex_snippet() {
    println!("Add the following to ~/.codex/config.toml (merge with existing content):\n");
    println!("[[hooks.Stop]]");
    println!("  [[hooks.Stop.hooks]]");
    println!("  type = \"command\"");
    println!("  command = \"{}\"", codex_hook_command());
}

// --- canonical JSON for change-detection (sorted keys, like json.dumps) ------

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), sorted(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn canonical(v: &Value) -> String {
    serde_json::to_string(&sorted(v)).unwrap_or_default()
}

// --- Claude Code (settings.json) --------------------------------------------

fn claude_install(settings_path: Option<&str>) {
    let path = settings_path
        .map(PathBuf::from)
        .unwrap_or_else(claude_settings_path);

    let mut settings: Value = json!({});
    let original_text = fs::read_to_string(&path).ok();
    if let Some(text) = &original_text {
        match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => settings = v,
            Ok(_) => {
                ui::bad(&format!("{} is not a JSON object; merge by hand:\n", path.display()));
                print_snippet();
                std::process::exit(1);
            }
            Err(e) => {
                ui::bad(&format!("could not parse {}: {e}", path.display()));
                println!("  Fix the file (or merge by hand using the snippet below)\n");
                print_snippet();
                std::process::exit(1);
            }
        }
    }

    let before = canonical(&settings);
    let obj = settings.as_object_mut().unwrap();

    // Ensure a hooks object.
    if !obj.get("hooks").map(|h| h.is_object()).unwrap_or(false) {
        obj.insert("hooks".into(), json!({}));
    }
    let hooks = obj.get_mut("hooks").unwrap().as_object_mut().unwrap();

    // Sweep Lystn entries out of EVERY event (removes legacy UserPromptSubmit).
    let events: Vec<String> = hooks.keys().cloned().collect();
    for event in events {
        let Some(arr) = hooks.get(&event).and_then(|e| e.as_array()) else {
            continue;
        };
        let kept: Vec<Value> = arr.iter().filter(|e| !is_lystn_entry(e)).cloned().collect();
        if kept.is_empty() {
            hooks.remove(&event);
        } else {
            hooks.insert(event, Value::Array(kept));
        }
    }

    // Add our Stop hook.
    if let Some(stop_entries) = lystn_hooks().get("Stop").and_then(|s| s.as_array()) {
        let entry = hooks.entry("Stop".to_string()).or_insert_with(|| json!([]));
        if let Some(arr) = entry.as_array_mut() {
            arr.extend(stop_entries.iter().cloned());
        }
    }

    // Add our status line — only if there's none, or the existing one is ours.
    let install_sl = match obj.get("statusLine") {
        None => true,
        Some(sl) => is_lystn_statusline(sl),
    };
    if install_sl {
        obj.insert(
            "statusLine".into(),
            json!({
                "type": "command",
                "command": format!("{} statusline", lystn_exe()),
                "refreshInterval": 1
            }),
        );
    }

    let changed = canonical(&settings) != before;
    if !changed {
        ui::ok(&format!("already wired up in {}", path.display()));
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Some(text) = &original_text {
        let _ = fs::write(with_suffix(&path, "json.bak"), text);
    }
    let out = serde_json::to_string_pretty(&settings).unwrap_or_default() + "\n";
    if let Err(e) = fs::write(&path, out) {
        ui::bad(&format!("could not write {}: {e}", path.display()));
        std::process::exit(1);
    }
    ui::ok(&format!("hooks merged into {}", path.display()));
    let bak = with_suffix(&path, "json.bak");
    if bak.exists() {
        ui::row("backup", &bak.display().to_string());
    }
    println!("  Restart Claude Code (or start a new session) to pick it up.");
}

fn claude_uninstall(settings_path: Option<&str>) {
    let path = settings_path
        .map(PathBuf::from)
        .unwrap_or_else(claude_settings_path);
    if !path.exists() {
        ui::ok(&format!("nothing to do — {} does not exist", path.display()));
        return;
    }
    let original_text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            ui::bad(&format!("could not read {}: {e}", path.display()));
            std::process::exit(1);
        }
    };
    let mut settings: Value = match serde_json::from_str(&original_text) {
        Ok(v) => v,
        Err(e) => {
            ui::bad(&format!(
                "could not parse {}: {e} — remove the Lystn hooks by hand",
                path.display()
            ));
            std::process::exit(1);
        }
    };
    let Some(obj) = settings.as_object_mut() else {
        ui::ok("no Lystn config found — nothing removed");
        return;
    };

    let mut changed = false;
    if let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let events: Vec<String> = hooks.keys().cloned().collect();
        for event in events {
            let Some(arr) = hooks.get(&event).and_then(|e| e.as_array()) else {
                continue;
            };
            let kept: Vec<Value> = arr.iter().filter(|e| !is_lystn_entry(e)).cloned().collect();
            if kept.len() != arr.len() {
                changed = true;
                if kept.is_empty() {
                    hooks.remove(&event);
                } else {
                    hooks.insert(event, Value::Array(kept));
                }
            }
        }
    }
    if obj
        .get("hooks")
        .and_then(|h| h.as_object())
        .map(|h| h.is_empty())
        .unwrap_or(false)
    {
        obj.remove("hooks");
    }
    if obj
        .get("statusLine")
        .map(is_lystn_statusline)
        .unwrap_or(false)
    {
        obj.remove("statusLine");
        changed = true;
    }

    if !changed {
        ui::ok("no Lystn config found — nothing removed");
        return;
    }

    let _ = fs::write(with_suffix(&path, "json.bak"), &original_text);
    let out = serde_json::to_string_pretty(&settings).unwrap_or_default() + "\n";
    let _ = fs::write(&path, out);
    ui::ok(&format!("Lystn hooks removed from {}", path.display()));
    ui::row("backup", &with_suffix(&path, "json.bak").display().to_string());
}

// --- Codex (config.toml) -----------------------------------------------------

fn codex_table_is_lystn(table: &Table) -> bool {
    let Some(inner) = table.get("hooks").and_then(|i| i.as_array_of_tables()) else {
        return false;
    };
    inner.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(is_lystn_command)
            .unwrap_or(false)
    })
}

fn codex_install(config_path: Option<&str>, print_only: bool) {
    if print_only {
        print_codex_snippet();
        return;
    }
    let path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(codex_config_path);

    let original_text = fs::read_to_string(&path).ok();
    let mut doc = match &original_text {
        Some(text) => match text.parse::<DocumentMut>() {
            Ok(d) => d,
            Err(e) => {
                ui::bad(&format!("could not parse {}: {e}", path.display()));
                println!("  Fix the file (or merge by hand using the snippet below)\n");
                print_codex_snippet();
                std::process::exit(1);
            }
        },
        None => DocumentMut::new(),
    };

    let before = doc.to_string();

    // Ensure doc["hooks"] is a table.
    if !doc.get("hooks").map(|i| i.is_table()).unwrap_or(false) {
        doc.insert("hooks", Item::Table(Table::new()));
    }
    let hooks = doc["hooks"].as_table_mut().unwrap();

    // Ensure hooks["Stop"] is an array-of-tables.
    if !hooks
        .get("Stop")
        .map(|i| i.is_array_of_tables())
        .unwrap_or(false)
    {
        hooks.insert("Stop", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let stop = hooks["Stop"].as_array_of_tables_mut().unwrap();

    // Sweep our previous entries (refreshes a moved path; avoids dupes).
    let kept: Vec<Table> = stop
        .iter()
        .filter(|t| !codex_table_is_lystn(t))
        .cloned()
        .collect();
    stop.clear();
    for t in kept {
        stop.push(t);
    }

    // Build [[hooks.Stop]] -> [[hooks.Stop.hooks]] type/command/statusMessage.
    let mut cmd_tbl = Table::new();
    cmd_tbl.insert("type", value("command"));
    cmd_tbl.insert("command", value(codex_hook_command()));
    cmd_tbl.insert("statusMessage", value(STATUS_MESSAGE));
    let mut inner_aot = ArrayOfTables::new();
    inner_aot.push(cmd_tbl);
    let mut entry = Table::new();
    entry.insert("hooks", Item::ArrayOfTables(inner_aot));
    stop.push(entry);

    let changed = doc.to_string() != before;
    if !changed {
        ui::ok(&format!("already wired up in {}", path.display()));
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Some(text) = &original_text {
        let _ = fs::write(with_suffix(&path, "toml.bak"), text);
    }
    if let Err(e) = fs::write(&path, doc.to_string()) {
        ui::bad(&format!("could not write {}: {e}", path.display()));
        std::process::exit(1);
    }
    ui::ok(&format!("Codex hook merged into {}", path.display()));
    let bak = with_suffix(&path, "toml.bak");
    if bak.exists() {
        ui::row("backup", &bak.display().to_string());
    }
    println!("  Restart Codex (or start a new session) to pick it up.");
}

fn codex_uninstall(config_path: Option<&str>) {
    let path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(codex_config_path);
    if !path.exists() {
        ui::ok(&format!("nothing to do — {} does not exist", path.display()));
        return;
    }
    let original_text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            ui::bad(&format!("could not read {}: {e}", path.display()));
            std::process::exit(1);
        }
    };
    let mut doc = match original_text.parse::<DocumentMut>() {
        Ok(d) => d,
        Err(e) => {
            ui::bad(&format!(
                "could not parse {}: {e} — remove the Lystn hook by hand",
                path.display()
            ));
            std::process::exit(1);
        }
    };

    let mut changed = false;
    if let Some(hooks) = doc.get_mut("hooks").and_then(|i| i.as_table_mut()) {
        if let Some(stop) = hooks.get_mut("Stop").and_then(|i| i.as_array_of_tables_mut()) {
            let kept: Vec<Table> = stop
                .iter()
                .filter(|t| !codex_table_is_lystn(t))
                .cloned()
                .collect();
            if kept.len() != stop.len() {
                changed = true;
                stop.clear();
                for t in kept {
                    stop.push(t);
                }
            }
            if stop.is_empty() {
                hooks.remove("Stop");
            }
        }
        if hooks.is_empty() {
            doc.remove("hooks");
        }
    }

    if !changed {
        ui::ok("no Lystn hook found — nothing removed");
        return;
    }

    let _ = fs::write(with_suffix(&path, "toml.bak"), &original_text);
    let _ = fs::write(&path, doc.to_string());
    ui::ok(&format!("Lystn hook removed from {}", path.display()));
    ui::row("backup", &with_suffix(&path, "toml.bak").display().to_string());
}

// --- public entry points -----------------------------------------------------

pub fn install(
    print_only: bool,
    settings_path: Option<&str>,
    codex: bool,
    config_path: Option<&str>,
) {
    ui::banner("install");
    if codex || config_path.is_some() {
        codex_install(config_path, print_only);
        return;
    }
    if print_only {
        print_snippet();
        println!();
        print_codex_snippet();
        return;
    }
    // Default: wire BOTH Claude Code and Codex (Codex config created if absent).
    claude_install(settings_path);
    println!();
    if codex_detected() {
        ui::grey("  Codex detected — wiring app/CLI/IDE up too:");
    } else {
        ui::grey("  Wiring Codex too (~/.codex/config.toml) so it's ready when you use it:");
    }
    codex_install(None, false);
}

pub fn uninstall(settings_path: Option<&str>, codex: bool, config_path: Option<&str>) {
    ui::banner("uninstall");
    if codex || config_path.is_some() {
        codex_uninstall(config_path);
        return;
    }
    claude_uninstall(settings_path);
    if codex_detected() {
        println!();
        ui::grey("  Codex detected — removing from it too:");
        codex_uninstall(None);
    }
    println!();
    ui::grey(
        "  Sorry to see you go. Everything Lystn added is removed (hooks + status \
         line); your .bak backups remain.",
    );
    ui::grey("  Reinstall anytime:  npm i -g lystn-cli");
}

/// Replace the file extension to build the `.bak` sibling (e.g. settings.json ->
/// settings.json.bak). Python uses `with_suffix(".json.bak")` which REPLACES the
/// existing extension, so settings.json -> settings.json.bak only because the
/// stem keeps ".json"? No — Python `Path("settings.json").with_suffix(".json.bak")`
/// yields `settings.json.bak`. We do the same: strip the last extension, append.
fn with_suffix(path: &std::path::Path, new_ext: &str) -> PathBuf {
    path.with_extension(new_ext)
}
