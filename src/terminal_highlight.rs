//! Syntax highlighting for CSS printed in error messages.
//!
//! Port of `lib/terminal-highlight.js`, with `picocolors` replaced by raw ANSI
//! escapes.

use crate::input::Input;
use crate::tokenize::{TokenKind, Tokenizer, TokenizerOptions};

/// Honours `NO_COLOR`/`FORCE_COLOR` and checks for a TTY, like `picocolors`.
pub fn is_color_supported() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("FORCE_COLOR").is_some() {
        return true;
    }
    if std::env::var("TERM").map(|term| term == "dumb").unwrap_or(false) {
        return false;
    }
    is_terminal()
}

fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

const GRAY: &str = "\u{1b}[90m";
const RED: &str = "\u{1b}[31m";
const GREEN: &str = "\u{1b}[32m";
const YELLOW: &str = "\u{1b}[33m";
const MAGENTA: &str = "\u{1b}[35m";
const CYAN: &str = "\u{1b}[36m";
const RESET: &str = "\u{1b}[39m";

/// The token colour theme, as `HIGHLIGHT_THEME` in the JS source.
fn theme(kind: HighlightKind) -> Option<&'static str> {
    Some(match kind {
        HighlightKind::AtWord | HighlightKind::Brackets | HighlightKind::Call => CYAN,
        HighlightKind::Class => YELLOW,
        HighlightKind::Comment => GRAY,
        HighlightKind::Hash => MAGENTA,
        HighlightKind::Paren => CYAN,
        HighlightKind::Punctuation => YELLOW,
        HighlightKind::Str => GREEN,
        HighlightKind::Plain => return None,
    })
}

#[derive(Clone, Copy)]
enum HighlightKind {
    AtWord,
    Brackets,
    Call,
    Class,
    Comment,
    Hash,
    Paren,
    Plain,
    Punctuation,
    Str,
}

/// Wraps CSS in ANSI colour escapes for terminal output.
pub fn highlight(css: &str) -> String {
    let input = Input::from_css(css);
    let mut tokenizer = Tokenizer::new(
        &input,
        TokenizerOptions {
            ignore_errors: true,
        },
    );

    let mut result = String::new();
    while !tokenizer.end_of_file() {
        let Ok(Some(token)) = tokenizer.next_token(false) else {
            break;
        };

        let mut kind = match token.kind {
            TokenKind::AtWord => HighlightKind::AtWord,
            TokenKind::Brackets => HighlightKind::Brackets,
            TokenKind::Comment => HighlightKind::Comment,
            TokenKind::Str => HighlightKind::Str,
            TokenKind::Colon | TokenKind::Semicolon => HighlightKind::Punctuation,
            TokenKind::OpenSquare
            | TokenKind::CloseSquare
            | TokenKind::OpenCurly
            | TokenKind::CloseCurly => HighlightKind::Punctuation,
            TokenKind::OpenParen | TokenKind::CloseParen => HighlightKind::Paren,
            TokenKind::Word => {
                if token.content.starts_with('.') {
                    HighlightKind::Class
                } else if token.content.starts_with('#') {
                    HighlightKind::Hash
                } else {
                    HighlightKind::Plain
                }
            }
            TokenKind::Space => HighlightKind::Plain,
        };

        // A word or at-word directly followed by a paren is a function call.
        if !matches!(kind, HighlightKind::Class | HighlightKind::Hash) && !tokenizer.end_of_file() {
            if let Ok(Some(next)) = tokenizer.next_token(false) {
                tokenizer.back(next);
                if matches!(next.kind, TokenKind::Brackets | TokenKind::OpenParen) {
                    kind = HighlightKind::Call;
                }
            }
        }

        match theme(kind) {
            Some(color) => {
                // `split(/\r?\n/).join('\n')` in JS, so a CR is dropped.
                let parts: Vec<String> = token
                    .content
                    .split('\n')
                    .map(|line| line.strip_suffix('\r').unwrap_or(line))
                    .map(|line| format!("{}{}{}", color, line, RESET))
                    .collect();
                result.push_str(&parts.join("\n"));
            }
            None => result.push_str(token.content),
        }
    }

    result
}

/// Unused today, kept so the palette stays in one place.
#[allow(dead_code)]
pub(crate) const RED_BOLD: &str = RED;
