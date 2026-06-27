//! Tiny console-styling helpers mirroring `cli/src/lystn/cli.py` (_banner, _ok,
//! _bad, _row). ANSI colors, ASCII-only glyphs (some Windows consoles use legacy
//! codepages that can't encode fancy symbols).

const MAGENTA: &str = "\u{1b}[35;1m";
const GREEN: &str = "\u{1b}[32m";
const RED: &str = "\u{1b}[31m";
const CYAN: &str = "\u{1b}[36m";
const GREY: &str = "\u{1b}[90m";
const RESET: &str = "\u{1b}[0m";

pub fn banner(subtitle: &str) {
    if subtitle.is_empty() {
        println!("{MAGENTA}== Lystn =={RESET}");
    } else {
        println!("{MAGENTA}== Lystn =={RESET}  {GREY}{subtitle}{RESET}");
    }
}

pub fn ok(msg: &str) {
    println!("  {GREEN}[ok] {msg}{RESET}");
}

pub fn bad(msg: &str) {
    println!("  {RED}[x] {msg}{RESET}");
}

pub fn row(label: &str, value: &str) {
    println!("  {CYAN}{label:<9}{RESET}{value}");
}

pub fn grey(msg: &str) {
    println!("{GREY}{msg}{RESET}");
}
