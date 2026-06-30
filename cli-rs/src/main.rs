//! Lystn CLI (Rust) — `lystn <command> ...`.
//!
//! Phase 2 of the Rust client migration (see
//! `docs/architecture/RUST-CLIENT-MIGRATION.md`). The full command surface is
//! implemented and mirrors the Python client (`cli/src/lystn/cli.py`): `hook`,
//! `install`/`uninstall` (Claude Code settings.json + Codex config.toml),
//! `login`, `config` (show/set), `mute`/`unmute`, `speed`, `volume`,
//! `statusline`.

mod api;
mod audio;
mod clean;
mod commands;
mod config;
mod hook;
mod install;
mod login;
mod statusline;
mod test;
mod ui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "lystn",
    version,
    about = "Listen to AI coding assistant responses."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Stop-hook entry point for Claude Code / Codex. Reads JSON from stdin.
    Hook {
        /// Assistant label sent as X-Lystn-Source (default: env LYSTN_SOURCE, else 'claude').
        #[arg(long)]
        source: Option<String>,
    },
    /// Wire Lystn into your AI assistants (Claude Code + Codex).
    #[command(alias = "install")]
    Wire {
        /// Print the hooks snippet(s) instead of writing any config.
        #[arg(long = "print", alias = "print-only")]
        print_only: bool,
        /// Path to Claude Code settings.json (default: ~/.claude/settings.json).
        #[arg(long = "settings")]
        settings_path: Option<String>,
        /// Wire ONLY local Codex surfaces (~/.codex/config.toml).
        #[arg(long)]
        codex: bool,
        /// Path to Codex config.toml (default: $CODEX_HOME or ~/.codex/config.toml). Implies --codex.
        #[arg(long = "config")]
        config_path: Option<String>,
    },
    /// Remove Lystn's hooks (Claude Code + Codex).
    #[command(alias = "uninstall")]
    Unwire {
        /// Path to Claude Code settings.json (default: ~/.claude/settings.json).
        #[arg(long = "settings")]
        settings_path: Option<String>,
        /// Remove ONLY from local Codex surfaces.
        #[arg(long)]
        codex: bool,
        /// Path to Codex config.toml. Implies --codex.
        #[arg(long = "config")]
        config_path: Option<String>,
    },
    /// Sign in and store your API key.
    Login {
        /// Server URL override.
        #[arg(long)]
        server: Option<String>,
    },
    /// Sign out — remove the saved API key from this machine.
    Logout,
    /// View or change saved settings.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Stop Lystn from speaking.
    Mute,
    /// Resume speaking after `lystn mute`.
    Unmute,
    /// Set playback speed (0.5-3.0).
    Speed {
        /// New speed; omit to show the current value.
        value: Option<String>,
    },
    /// Set volume (0-100).
    Volume {
        /// New volume; omit to show the current value.
        value: Option<String>,
    },
    /// Print the Claude Code statusline string.
    Statusline,
    /// Speak a built-in sample to test audio + a language (e.g. test, test tr, test fr).
    Test {
        /// Language: en (default), tr, fr, es, it, pt, hi, zh.
        lang: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the current config.
    Show,
    /// Set KEY to VALUE (server, api_key, voice, device, speed, muted, volume).
    Set {
        key: String,
        value: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Hook { source } => hook::run(source),
        Commands::Wire {
            print_only,
            settings_path,
            codex,
            config_path,
        } => install::install(
            print_only,
            settings_path.as_deref(),
            codex,
            config_path.as_deref(),
        ),
        Commands::Unwire {
            settings_path,
            codex,
            config_path,
        } => install::uninstall(settings_path.as_deref(), codex, config_path.as_deref()),
        Commands::Login { server } => login::run(server.as_deref()),
        Commands::Logout => commands::logout(),
        Commands::Config { action } => match action {
            ConfigAction::Show => commands::config_show(),
            ConfigAction::Set { key, value } => commands::config_set(&key, &value),
        },
        Commands::Mute => commands::set_muted(true),
        Commands::Unmute => commands::set_muted(false),
        Commands::Speed { value } => commands::speed(value.as_deref()),
        Commands::Volume { value } => commands::volume(value.as_deref()),
        Commands::Statusline => statusline::run(),
        Commands::Test { lang } => test::run(lang),
    }
}
