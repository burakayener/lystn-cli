//! Local privacy pre-clean — runs on the USER'S machine before any text is sent.
//!
//! Strips the privacy-sensitive parts of a raw assistant reply — fenced/indented/
//! inline code, URLs, file paths and source filenames — so they never leave the
//! device. Only the plain prose we actually summarize + speak is transmitted.
//!
//! The server's CLEANER_1 still runs afterwards (idempotent) for the cosmetic
//! cleanup that isn't privacy-sensitive (markdown emphasis, emoji, mojibake,
//! spacing). These patterns deliberately MIRROR the server's CLEANER_1 so
//! re-cleaning is a no-op. We avoid regex lookaround (unsupported by the `regex`
//! crate) by capturing the leading whitespace on the unix-path rule instead.

use regex::Regex;
use std::sync::OnceLock;

struct Cleaner {
    fenced: Regex,
    indented: Regex,
    inline: Regex,
    url_http: Regex,
    url_www: Regex,
    win_path: Regex,
    unix_path: Regex,
    filename: Regex,
}

fn cleaner() -> &'static Cleaner {
    static C: OnceLock<Cleaner> = OnceLock::new();
    C.get_or_init(|| Cleaner {
        // ```...``` across lines -> spoken placeholder (matches the server).
        fenced: Regex::new(r"(?s)```.*?```").unwrap(),
        // 4-space indented code lines.
        indented: Regex::new(r"(?m)^    [^\n]+\n?").unwrap(),
        // `inline code`.
        inline: Regex::new(r"`[^`\n]+`").unwrap(),
        url_http: Regex::new(r"https?://\S+").unwrap(),
        url_www: Regex::new(r"www\.\S+").unwrap(),
        // Windows path: C:\... up to whitespace.
        win_path: Regex::new(r"[A-Za-z]:\\\S+").unwrap(),
        // Unix path preceded by whitespace (group 1 keeps the space; the server
        // uses a lookbehind `(?<=\s)/...` which the regex crate can't do).
        unix_path: Regex::new(r"(\s)/[\w./\-]+").unwrap(),
        // Bare source filenames (foo.py, app.tsx, ...).
        filename: Regex::new(
            r"\b[\w\-]+\.(?:py|ts|tsx|js|jsx|json|md|rs|go|java|cs|cpp|c|h|html|css|sh|bat|ps1|yaml|yml|toml|xml|sql|rb|php|swift|kt)\b",
        )
        .unwrap(),
    })
}

/// Strip code, URLs, paths and filenames from a raw reply. Returns the prose that
/// is safe to send. Trimmed; may be empty if the reply was nothing but code/paths.
pub fn pre_clean(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let c = cleaner();
    let mut t = c.fenced.replace_all(text, " code block omitted. ").into_owned();
    t = c.indented.replace_all(&t, "").into_owned();
    t = c.inline.replace_all(&t, "").into_owned();
    t = c.url_http.replace_all(&t, "").into_owned();
    t = c.url_www.replace_all(&t, "").into_owned();
    t = c.win_path.replace_all(&t, "").into_owned();
    t = c.unix_path.replace_all(&t, "${1}").into_owned();
    t = c.filename.replace_all(&t, "").into_owned();
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::pre_clean;

    #[test]
    fn strips_code_paths_urls() {
        let raw = "Fixed the bug in src/main.rs. See ```rust\nfn x(){}\n``` and \
                   visit https://example.com or C:\\Users\\me\\secret.txt — done.";
        let out = pre_clean(raw);
        assert!(!out.contains("fn x()"));
        assert!(!out.contains("https://"));
        assert!(!out.contains("C:\\Users"));
        assert!(!out.contains("main.rs"));
        assert!(out.contains("Fixed the bug"));
        assert!(out.contains("code block omitted"));
    }

    #[test]
    fn idempotent_on_clean_prose() {
        let prose = "I updated the function and it now returns the right value.";
        assert_eq!(pre_clean(prose), prose);
    }
}
