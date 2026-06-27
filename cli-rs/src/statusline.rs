//! `lystn statusline` — the branded animated Claude Code status line.
//!
//! Mirrors `cli/src/lystn/statusline.py`: Claude Code pipes session JSON on
//! stdin; we print ONE line with the Lystn mark, an animated voice glyph, and
//! the model. Each session deterministically gets one animation style (stable
//! within the session), and the frame advances + persists per session in the
//! temp dir. Pure text + ANSI — no dependencies, no network.

use std::io::Read;

/// Animation styles, keyed in the same order as Python's `_ORDER`.
const STYLES: &[(&str, &[&str])] = &[
    (
        "rings",
        &[
            "   \u{25cf}   ",
            "  (\u{25cf})  ",
            " ((\u{25cf})) ",
            "((\u{25ce} \u{25ce}))",
            "(\u{25cc}   \u{25cc})",
            " \u{25cc}   \u{25cc} ",
            "   \u{25cc}   ",
            "   \u{25cd}   ",
        ],
    ),
    (
        "wave",
        &[
            "\u{2581}\u{2582}\u{2583}",
            "\u{2582}\u{2583}\u{2584}",
            "\u{2583}\u{2584}\u{2585}",
            "\u{2584}\u{2585}\u{2586}",
            "\u{2585}\u{2586}\u{2587}",
            "\u{2586}\u{2587}\u{2586}",
            "\u{2587}\u{2586}\u{2585}",
            "\u{2585}\u{2584}\u{2583}",
            "\u{2583}\u{2582}\u{2581}",
            "\u{2582}\u{2581}\u{2582}",
        ],
    ),
    (
        "bars",
        &[
            "\u{2582}\u{2584}\u{2586}\u{2584}\u{2582}",
            "\u{2584}\u{2586}\u{2588}\u{2586}\u{2584}",
            "\u{2586}\u{2588}\u{2586}\u{2588}\u{2586}",
            "\u{2588}\u{2586}\u{2584}\u{2586}\u{2588}",
            "\u{2586}\u{2584}\u{2582}\u{2584}\u{2586}",
            "\u{2584}\u{2582}\u{2581}\u{2582}\u{2584}",
        ],
    ),
    (
        "pulse",
        &[
            " \u{00b7} ", " \u{25cc} ", " \u{25cb} ", " \u{25c9} ", " \u{25cf} ",
            " \u{25c9} ", " \u{25cb} ", " \u{25cc} ",
        ],
    ),
    (
        "aperture",
        &[
            " \u{25cc} ", " \u{25cd} ", " \u{25ce} ", " \u{25cf} ", " \u{25ce} ",
            " \u{25cd} ",
        ],
    ),
    (
        "eqring",
        &[
            "(\u{2582}\u{2587}\u{2585})",
            "(\u{2583}\u{2588}\u{2583})",
            "(\u{2585}\u{2587}\u{2582})",
            "(\u{2586}\u{2586}\u{2583})",
            "(\u{2585}\u{2584}\u{2585})",
            "(\u{2583}\u{2586}\u{2586})",
        ],
    ),
];

// 3-stop gradient across the rings (indigo -> violet -> teal). 256-color cube.
const GRADIENT: [u8; 4] = [99, 135, 141, 80];
const PURPLE: &str = "\u{1b}[38;5;99m";
const DIM: &str = "\u{1b}[2m";
const RESET: &str = "\u{1b}[0m";

/// Tint a frame: characters further from center shade toward teal.
fn colorize(frame: &str) -> String {
    let chars: Vec<char> = frame.chars().collect();
    let n = chars.len();
    let mid = if n > 0 { (n as f64 - 1.0) / 2.0 } else { 0.0 };
    let mut out = String::new();
    let mut prev: Option<u8> = None;
    for (i, &ch) in chars.iter().enumerate() {
        if ch == ' ' {
            out.push(' ');
            prev = None;
            continue;
        }
        let t = if mid != 0.0 {
            (i as f64 - mid).abs() / mid
        } else {
            0.0
        };
        let idx = ((t * GRADIENT.len() as f64) as usize).min(GRADIENT.len() - 1);
        let color = GRADIENT[idx];
        if Some(color) != prev {
            out.push_str(&format!("\u{1b}[38;5;{color}m"));
            prev = Some(color);
        }
        out.push(ch);
    }
    out.push_str(RESET);
    out
}

/// Advance + persist the animation frame for this session.
fn frame_index(session_id: &str, n: usize) -> usize {
    let path = std::env::temp_dir().join(format!("lystn-sl-{session_id}.frame"));
    let cur = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let next = (cur + 1) % n.max(1);
    let _ = std::fs::write(&path, next.to_string());
    next
}

pub fn run() {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);

    let session_id = data
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "x".to_string());
    let model = data
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // Style: an explicit CLI arg wins (e.g. `statusline eqring`); otherwise
    // default to the brand "eqring" style so the mark stays consistent.
    let style_idx = std::env::args()
        .nth(1)
        .and_then(|name| STYLES.iter().position(|(n, _)| *n == name))
        .unwrap_or_else(|| STYLES.iter().position(|(n, _)| *n == "eqring").unwrap_or(0));
    let (_name, frames) = STYLES[style_idx];
    let anim = colorize(frames[frame_index(&session_id, frames.len())]);
    let mut out = format!("{PURPLE}\u{1f50a} lystn{RESET} {anim}");
    if !model.is_empty() {
        out.push_str(&format!("  {DIM}{model}{RESET}"));
    }
    println!("{out}");
}
