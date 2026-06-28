//! `lystn test [lang]` — speak a built-in sample so you can verify audio and a
//! language's voice without opening your AI assistant. The server detects the
//! language from the phrase and routes to that language's native voice.
//!
//!   lystn test        -> English (your configured voice)
//!   lystn test tr     -> Turkish, test fr -> French, es/it/pt/hi/zh -> ...

use crate::api::{self, SpeakParams};
use crate::config::Config;

pub fn run(lang: Option<String>) {
    let lang = lang
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".to_string());
    let text = phrase(&lang);

    let cfg = Config::load();
    if cfg.api_key.as_deref().unwrap_or("").is_empty() {
        eprintln!("[lystn] No API key yet — run `lystn login` first, then `lystn test`.");
    }
    let voice = cfg.voice.clone();
    let session = session_id();

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[lystn] could not start runtime: {e}");
            std::process::exit(1);
        }
    };

    println!("Playing a {} test through Lystn…", lang_name(&lang));
    let result = rt.block_on(async {
        api::synthesize_and_play(SpeakParams {
            server: &cfg.server,
            api_key: cfg.api_key.as_deref(),
            text,
            voice: Some(&voice),
            label: None,
            terminal: None,
            source: "test",
            session: &session,
            speed: cfg.clamped_speed(),
            volume: cfg.volume_gain(),
        })
        .await
    });

    match result {
        Ok(()) => println!(
            "Done. If you heard nothing, check your output device — and run `lystn login` if you haven't."
        ),
        Err(e) => {
            eprintln!("[lystn] test failed: {e}");
            eprintln!("[lystn] If that's an auth error, run `lystn login` first.");
            std::process::exit(1);
        }
    }
}

/// A short, natural sample in each supported language. Kept under the server's
/// skip-LLM threshold so it's spoken as-is in the language's native voice.
fn phrase(lang: &str) -> &'static str {
    match lang {
        "tr" => "Merhaba! Ben Lystn. Yapay zekâ asistanının cevaplarını senin için sesli okuyacağım.",
        "fr" => "Bonjour ! Je suis Lystn. Je vais lire à voix haute les réponses de votre assistant IA.",
        "es" => "¡Hola! Soy Lystn. Leeré en voz alta las respuestas de tu asistente de inteligencia artificial.",
        "it" => "Ciao! Sono Lystn. Leggerò ad alta voce le risposte del tuo assistente di intelligenza artificiale.",
        "pt" => "Olá! Eu sou o Lystn. Vou ler em voz alta as respostas do seu assistente de inteligência artificial.",
        "hi" => "नमस्ते! मैं Lystn हूँ। मैं आपके ए आई असिस्टेंट के जवाब ज़ोर से पढ़कर सुनाऊँगा।",
        "zh" => "你好，我是 Lystn。我会把你的 AI 助手的回复大声朗读出来。",
        _ => "Hi! This is Lystn. I'll read your AI assistant's replies out loud, so you can listen instead of read.",
    }
}

fn lang_name(lang: &str) -> &'static str {
    match lang {
        "tr" => "Turkish",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "hi" => "Hindi",
        "zh" => "Chinese",
        _ => "English",
    }
}

fn session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("test-{nanos:032x}-{:08x}", std::process::id())
}
