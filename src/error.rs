//! `CssSyntaxError` — the error type every fallible PostCSS operation returns.
//!
//! Port of `lib/css-syntax-error.js`.

use std::fmt;

use crate::terminal_highlight::{highlight, is_color_supported};

/// Where in the original input an error happened.
///
/// Mirrors the `error.input` object in PostCSS. `offset` is a UTF-8 byte
/// offset; `column`s count characters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorInput {
    /// 1-based character column of the error start.
    pub column: usize,
    /// 1-based character column of the error end.
    pub end_column: Option<usize>,
    /// 1-based line of the error end.
    pub end_line: Option<usize>,
    /// Byte offset of the error end.
    pub end_offset: Option<usize>,
    /// Path of the file, when known.
    pub file: Option<String>,
    /// 1-based line of the error start.
    pub line: usize,
    /// Byte offset of the error start.
    pub offset: usize,
    /// The CSS the error was found in.
    pub source: String,
    /// `file:` URL of the file, when known.
    pub url: Option<String>,
}

/// The CSS parser/plugin error type.
///
/// ```
/// # use postcss::parse;
/// let err = parse("a {").unwrap_err();
/// assert_eq!(err.reason, "Unclosed block");
/// assert_eq!(err.line, Some(1));
/// assert_eq!(
///     err.to_string(),
///     "CssSyntaxError: <css input>:1:1: Unclosed block\n\n> 1 | a {\n    | ^\n"
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssSyntaxError {
    /// Always `"CssSyntaxError"`. Kept for parity with the JS `name` field.
    pub name: &'static str,
    /// Error message without position information.
    pub reason: String,
    /// Absolute path to the broken file, when known.
    pub file: Option<String>,
    /// Source code of the broken file.
    pub source: Option<String>,
    /// Name of the plugin that threw, when known.
    pub plugin: Option<String>,
    /// 1-based line of the error start.
    pub line: Option<usize>,
    /// 1-based column of the error start.
    pub column: Option<usize>,
    /// 1-based line of the error end.
    pub end_line: Option<usize>,
    /// 1-based column of the error end (exclusive).
    pub end_column: Option<usize>,
    /// Full message: `plugin: file:line:column: reason`.
    pub message: String,
    /// Position of the error inside the *original* input, before source maps.
    pub input: Option<ErrorInput>,
}

/// Start/end position pair accepted by the error constructors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pos {
    pub line: usize,
    pub column: usize,
}

impl CssSyntaxError {
    /// Creates an error with no position information.
    pub fn new(message: impl Into<String>) -> Self {
        let mut err = CssSyntaxError {
            name: "CssSyntaxError",
            reason: message.into(),
            file: None,
            source: None,
            plugin: None,
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            message: String::new(),
            input: None,
        };
        err.set_message();
        err
    }

    pub(crate) fn with_position(
        message: impl Into<String>,
        start: Option<Pos>,
        end: Option<Pos>,
        source: Option<String>,
        file: Option<String>,
        plugin: Option<String>,
    ) -> Self {
        let mut err = CssSyntaxError {
            name: "CssSyntaxError",
            reason: message.into(),
            file,
            source,
            plugin,
            line: start.map(|p| p.line),
            column: start.map(|p| p.column),
            end_line: end.map(|p| p.line),
            end_column: end.map(|p| p.column),
            message: String::new(),
            input: None,
        };
        err.set_message();
        err
    }

    /// Rebuilds [`CssSyntaxError::message`] from the current fields.
    ///
    /// Call this after changing `plugin`, `file`, `line`, `column` or `reason`.
    pub fn set_message(&mut self) {
        let mut message = match &self.plugin {
            Some(plugin) => format!("{}: ", plugin),
            None => String::new(),
        };
        message.push_str(self.file.as_deref().unwrap_or("<css input>"));
        if let (Some(line), Some(column)) = (self.line, self.column) {
            message.push_str(&format!(":{}:{}", line, column));
        }
        message.push_str(": ");
        message.push_str(&self.reason);
        self.message = message;
    }

    pub(crate) fn set_plugin(&mut self, plugin: &str) {
        if self.plugin.is_none() {
            self.plugin = Some(plugin.to_string());
            self.set_message();
        }
    }

    /// Returns a few lines of CSS around the error, with the error position
    /// marked by a caret.
    ///
    /// `color` selects ANSI colors; `None` auto-detects terminal support.
    pub fn show_source_code(&self, color: Option<bool>) -> String {
        let Some(css) = &self.source else {
            return String::new();
        };
        let Some(error_line) = self.line else {
            return String::new();
        };
        let color = color.unwrap_or_else(is_color_supported);

        let mark = |text: &str| {
            if color {
                format!("\u{1b}[1m\u{1b}[31m{}\u{1b}[39m\u{1b}[22m", text)
            } else {
                text.to_string()
            }
        };
        let aside = |text: &str| {
            if color {
                format!("\u{1b}[90m{}\u{1b}[39m", text)
            } else {
                text.to_string()
            }
        };
        let paint = |text: &str| {
            if color {
                highlight(text)
            } else {
                text.to_string()
            }
        };

        let lines: Vec<&str> = split_lines(css);
        let start = error_line.saturating_sub(3);
        let end = (error_line + 2).min(lines.len());
        let max_width = end.to_string().len();
        let column = self.column.unwrap_or(1);

        let mut out: Vec<String> = Vec::new();
        for (index, line) in lines[start..end].iter().enumerate() {
            let number = start + 1 + index;
            // `(' ' + number).slice(-maxWidth)` in JS: at most one pad space.
            let padded = format!(" {}", number);
            let padded = last_chars(&padded, max_width);
            let gutter = format!(" {} | ", padded);

            if number != error_line {
                out.push(format!(" {}{}", aside(&gutter), paint(line)));
                continue;
            }

            let blank_gutter = aside(&gutter.replace(|c: char| c.is_ascii_digit(), " "));
            let line_chars = line.chars().count();

            if line_chars > 160 {
                let padding = 20;
                let sub_start = column.saturating_sub(padding);
                let sub_end = (column + padding).max(self.end_column.unwrap_or(column) + padding);
                let sub_line = char_slice(line, sub_start, sub_end);
                let prefix = char_slice(line, 0, (column - 1).min(padding - 1));
                let spacing = format!("{}{}", blank_gutter, blank_out(&prefix));
                out.push(format!(
                    "{}{}{}\n {}{}",
                    mark(">"),
                    aside(&gutter),
                    paint(&sub_line),
                    spacing,
                    mark("^")
                ));
            } else {
                let prefix = char_slice(line, 0, column - 1);
                let spacing = format!("{}{}", blank_gutter, blank_out(&prefix));
                out.push(format!(
                    "{}{}{}\n {}{}",
                    mark(">"),
                    aside(&gutter),
                    paint(line),
                    spacing,
                    mark("^")
                ));
            }
        }

        out.join("\n")
    }
}

impl fmt::Display for CssSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.show_source_code(None);
        if code.is_empty() {
            write!(f, "{}: {}", self.name, self.message)
        } else {
            write!(f, "{}: {}\n\n{}\n", self.name, self.message, code)
        }
    }
}

impl std::error::Error for CssSyntaxError {}

/// Splits on `\r?\n`, like the JS `css.split(/\r?\n/)`.
fn split_lines(css: &str) -> Vec<&str> {
    css.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// Everything except tabs becomes a space, so the caret lines up.
fn blank_out(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect()
}

fn last_chars(text: &str, count: usize) -> String {
    let len = text.chars().count();
    text.chars().skip(len.saturating_sub(count)).collect()
}

fn char_slice(text: &str, from: usize, to: usize) -> String {
    text.chars().skip(from).take(to.saturating_sub(from)).collect()
}
