//! CSS tokenizer.
//!
//! Port of `lib/tokenize.js`. The scanner walks raw bytes: every character it
//! branches on is ASCII, and UTF-8 never encodes an ASCII byte inside a
//! multi-byte sequence, so byte scanning is safe for any valid UTF-8 input.
//!
//! Positions are UTF-8 byte offsets into the input.

use crate::error::CssSyntaxError;
use crate::input::Input;

const SINGLE_QUOTE: u8 = b'\'';
const DOUBLE_QUOTE: u8 = b'"';
const BACKSLASH: u8 = b'\\';
const SLASH: u8 = b'/';
const NEWLINE: u8 = b'\n';
const SPACE: u8 = b' ';
const FEED: u8 = 0x0c;
const TAB: u8 = b'\t';
const CR: u8 = b'\r';
const OPEN_SQUARE: u8 = b'[';
const CLOSE_SQUARE: u8 = b']';
const OPEN_PARENTHESES: u8 = b'(';
const CLOSE_PARENTHESES: u8 = b')';
const OPEN_CURLY: u8 = b'{';
const CLOSE_CURLY: u8 = b'}';
const SEMICOLON: u8 = b';';
const ASTERISK: u8 = b'*';
const COLON: u8 = b':';
const AT: u8 = b'@';

/// Token kinds produced by the tokenizer.
///
/// [`TokenKind::as_str`] returns the exact string PostCSS uses, so ports of
/// upstream tests can compare against the same literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// `@media`, `@import` — an at-rule name.
    AtWord,
    /// A balanced `(…)` run kept as one token.
    Brackets,
    /// `}`
    CloseCurly,
    /// `)`
    CloseParen,
    /// `]`
    CloseSquare,
    /// `:`
    Colon,
    /// `/* … */`
    Comment,
    /// `{`
    OpenCurly,
    /// `(` that could not be balanced into `Brackets`.
    OpenParen,
    /// `[`
    OpenSquare,
    /// `;`
    Semicolon,
    /// A run of whitespace.
    Space,
    /// A quoted string.
    Str,
    /// Anything else: identifiers, numbers, escapes.
    Word,
}

impl TokenKind {
    /// The exact type string PostCSS uses for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::AtWord => "at-word",
            TokenKind::Brackets => "brackets",
            TokenKind::CloseCurly => "}",
            TokenKind::CloseParen => ")",
            TokenKind::CloseSquare => "]",
            TokenKind::Colon => ":",
            TokenKind::Comment => "comment",
            TokenKind::OpenCurly => "{",
            TokenKind::OpenParen => "(",
            TokenKind::OpenSquare => "[",
            TokenKind::Semicolon => ";",
            TokenKind::Space => "space",
            TokenKind::Str => "string",
            TokenKind::Word => "word",
        }
    }
}

/// A single token: kind, source text, and the offsets of its first and last
/// byte.
///
/// `space` tokens carry no offsets and control characters carry only a start,
/// matching the variable-length arrays PostCSS produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    /// The token type.
    pub kind: TokenKind,
    /// The token's source text.
    pub content: &'a str,
    /// Byte offset of the token's first byte.
    pub start: Option<usize>,
    /// Byte offset of the token's last byte.
    pub end: Option<usize>,
}

impl<'a> Token<'a> {
    /// Start offset, or 0 when the token carries none.
    pub fn start_or_zero(&self) -> usize {
        self.start.unwrap_or(0)
    }

    /// `token[3] || token[2]`: the last offset of the token, falling back to
    /// its start. A zero `end` is treated as absent, as in JS — the fallback
    /// then yields the same 0.
    pub fn end_or_start(&self) -> Option<usize> {
        match self.end {
            Some(end) if end != 0 => Some(end),
            _ => self.start,
        }
    }

    /// Like [`Token::end_or_start`] but also rejects a zero result, matching
    /// the `if (pos)` check in `findLastWithPosition()`.
    pub fn truthy_position(&self) -> Option<usize> {
        self.end_or_start().filter(|&pos| pos != 0)
    }
}

/// Options for [`Tokenizer::new`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenizerOptions {
    /// Emit truncated tokens instead of failing on unclosed strings, comments
    /// and brackets.
    pub ignore_errors: bool,
}

/// Streaming CSS tokenizer.
pub struct Tokenizer<'a> {
    css: &'a str,
    bytes: &'a [u8],
    ignore: bool,
    length: usize,
    pos: usize,
    /// Word tokens, used only to check for `url(` before an opening paren.
    buffer: Vec<&'a str>,
    returned: Vec<Token<'a>>,
    last_bad_paren: i64,
    input: &'a Input,
}

impl<'a> Tokenizer<'a> {
    /// Creates a tokenizer over `input`.
    pub fn new(input: &'a Input, options: TokenizerOptions) -> Self {
        let css = input.css();
        Tokenizer {
            css,
            bytes: css.as_bytes(),
            ignore: options.ignore_errors,
            length: css.len(),
            pos: 0,
            buffer: Vec::new(),
            returned: Vec::new(),
            last_bad_paren: -1,
            input,
        }
    }

    /// Current scan offset.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// True when no tokens are pending and the input is exhausted.
    pub fn end_of_file(&self) -> bool {
        self.returned.is_empty() && self.pos >= self.length
    }

    /// Pushes a token back onto the stream.
    pub fn back(&mut self, token: Token<'a>) {
        self.returned.push(token);
    }

    /// Reads the next token.
    ///
    /// `ignore_unclosed` suppresses unclosed-construct errors for this token
    /// only, like the `{ ignoreUnclosed: true }` option in PostCSS.
    pub fn next_token(
        &mut self,
        ignore_unclosed: bool,
    ) -> Result<Option<Token<'a>>, CssSyntaxError> {
        if let Some(token) = self.returned.pop() {
            return Ok(Some(token));
        }
        if self.pos >= self.length {
            return Ok(None);
        }

        let mut code = self.bytes[self.pos];
        let token;

        match code {
            NEWLINE | SPACE | TAB | CR | FEED => {
                let mut next = self.pos;
                loop {
                    next += 1;
                    code = self.at(next);
                    if !matches!(code, SPACE | NEWLINE | TAB | CR | FEED) {
                        break;
                    }
                }
                token = Token {
                    kind: TokenKind::Space,
                    content: &self.css[self.pos..next],
                    start: None,
                    end: None,
                };
                self.pos = next - 1;
            }

            OPEN_SQUARE | CLOSE_SQUARE | OPEN_CURLY | CLOSE_CURLY | COLON | SEMICOLON
            | CLOSE_PARENTHESES => {
                let kind = match code {
                    OPEN_SQUARE => TokenKind::OpenSquare,
                    CLOSE_SQUARE => TokenKind::CloseSquare,
                    OPEN_CURLY => TokenKind::OpenCurly,
                    CLOSE_CURLY => TokenKind::CloseCurly,
                    COLON => TokenKind::Colon,
                    SEMICOLON => TokenKind::Semicolon,
                    _ => TokenKind::CloseParen,
                };
                token = Token {
                    kind,
                    content: &self.css[self.pos..self.pos + 1],
                    start: Some(self.pos),
                    end: None,
                };
            }

            OPEN_PARENTHESES => {
                let prev = self.buffer.pop().unwrap_or("");
                let n = self.at(self.pos + 1);
                if prev == "url"
                    && n != SINGLE_QUOTE
                    && n != DOUBLE_QUOTE
                    && n != SPACE
                    && n != NEWLINE
                    && n != TAB
                    && n != FEED
                    && n != CR
                {
                    let mut next = self.pos;
                    let mut escaped;
                    loop {
                        escaped = false;
                        match self.index_of(b')', next + 1) {
                            Some(found) => next = found,
                            None => {
                                if self.ignore || ignore_unclosed {
                                    next = self.pos;
                                    break;
                                } else {
                                    return Err(self.unclosed("bracket"));
                                }
                            }
                        }
                        let mut escape_pos = next;
                        while escape_pos > 0 && self.at(escape_pos - 1) == BACKSLASH {
                            escape_pos -= 1;
                            escaped = !escaped;
                        }
                        if !escaped {
                            break;
                        }
                    }

                    token = Token {
                        kind: TokenKind::Brackets,
                        content: &self.css[self.pos..next + 1],
                        start: Some(self.pos),
                        end: Some(next),
                    };
                    self.pos = next;
                } else if (self.pos as i64) <= self.last_bad_paren {
                    token = Token {
                        kind: TokenKind::OpenParen,
                        content: "(",
                        start: Some(self.pos),
                        end: None,
                    };
                } else {
                    match self.index_of(b')', self.pos + 1) {
                        None => {
                            self.last_bad_paren = self.length as i64;
                            token = Token {
                                kind: TokenKind::OpenParen,
                                content: "(",
                                start: Some(self.pos),
                                end: None,
                            };
                        }
                        Some(next) => {
                            let content = &self.css[self.pos..next + 1];
                            if is_bad_bracket(content) {
                                self.last_bad_paren = next as i64;
                                token = Token {
                                    kind: TokenKind::OpenParen,
                                    content: "(",
                                    start: Some(self.pos),
                                    end: None,
                                };
                            } else {
                                token = Token {
                                    kind: TokenKind::Brackets,
                                    content,
                                    start: Some(self.pos),
                                    end: Some(next),
                                };
                                self.pos = next;
                            }
                        }
                    }
                }
            }

            SINGLE_QUOTE | DOUBLE_QUOTE => {
                let quote = code;
                let mut next = self.pos;
                let mut escaped;
                let mut truncated = None;
                loop {
                    escaped = false;
                    match self.index_of(quote, next + 1) {
                        Some(found) => next = found,
                        None => {
                            if self.ignore || ignore_unclosed {
                                // JS keeps the quote plus one code unit; the
                                // closest well-formed equivalent is the quote
                                // plus one character, clamped to the input.
                                truncated = Some(self.char_end(self.pos + 1).min(self.length));
                                break;
                            } else {
                                return Err(self.unclosed("string"));
                            }
                        }
                    }
                    let mut escape_pos = next;
                    while escape_pos > 0 && self.at(escape_pos - 1) == BACKSLASH {
                        escape_pos -= 1;
                        escaped = !escaped;
                    }
                    if !escaped {
                        break;
                    }
                }

                if let Some(content_end) = truncated {
                    // JS slices past the end of the input here, so the token can
                    // end beyond the last byte.
                    next = self.pos + 1;
                    token = Token {
                        kind: TokenKind::Str,
                        content: &self.css[self.pos..content_end],
                        start: Some(self.pos),
                        end: Some(next),
                    };
                } else {
                    token = Token {
                        kind: TokenKind::Str,
                        content: &self.css[self.pos..next + 1],
                        start: Some(self.pos),
                        end: Some(next),
                    };
                }
                self.pos = next;
            }

            AT => {
                let next = match self.find_at_end(self.pos + 1) {
                    Some(index) => index - 1,
                    None => self.length - 1,
                };
                token = Token {
                    kind: TokenKind::AtWord,
                    content: &self.css[self.pos..self.char_end(next)],
                    start: Some(self.pos),
                    end: Some(next),
                };
                self.pos = next;
            }

            BACKSLASH => {
                let mut next = self.pos;
                let mut escape = true;
                while self.at(next + 1) == BACKSLASH {
                    next += 1;
                    escape = !escape;
                }
                code = self.at(next + 1);
                if escape
                    && code != SLASH
                    && code != SPACE
                    && code != NEWLINE
                    && code != TAB
                    && code != CR
                    && code != FEED
                {
                    next += 1;
                    if is_hex(self.at(next)) {
                        while is_hex(self.at(next + 1)) {
                            next += 1;
                        }
                        if self.at(next + 1) == SPACE {
                            next += 1;
                        }
                    }
                }

                token = Token {
                    kind: TokenKind::Word,
                    content: &self.css[self.pos..self.char_end(next)],
                    start: Some(self.pos),
                    end: Some(next),
                };
                self.pos = next;
            }

            _ => {
                if code == SLASH && self.at(self.pos + 1) == ASTERISK {
                    let next = match self.index_of_str("*/", self.pos + 2) {
                        Some(index) => index + 1,
                        None => {
                            if self.ignore || ignore_unclosed {
                                // JS: `next = css.length`, so the slice runs to
                                // the end of the input.
                                self.length
                            } else {
                                return Err(self.unclosed("comment"));
                            }
                        }
                    };
                    let end_exclusive = (next + 1).min(self.length);
                    token = Token {
                        kind: TokenKind::Comment,
                        content: &self.css[self.pos..end_exclusive],
                        start: Some(self.pos),
                        end: Some(next),
                    };
                    self.pos = next;
                } else {
                    let next = match self.find_word_end(self.pos + 1) {
                        Some(index) => index - 1,
                        None => self.length - 1,
                    };
                    token = Token {
                        kind: TokenKind::Word,
                        content: &self.css[self.pos..self.char_end(next)],
                        start: Some(self.pos),
                        end: Some(next),
                    };
                    self.buffer.push(token.content);
                    self.pos = next;
                }
            }
        }

        self.pos += 1;
        Ok(Some(token))
    }

    fn at(&self, index: usize) -> u8 {
        // Out of range reads model `charCodeAt()` returning NaN: a value that
        // matches none of the branches.
        if index < self.length {
            self.bytes[index]
        } else {
            0
        }
    }

    /// Smallest char boundary strictly greater than `index`, so a slice ending
    /// there never splits a character.
    fn char_end(&self, index: usize) -> usize {
        let mut end = (index + 1).min(self.length);
        while end < self.length && !self.css.is_char_boundary(end) {
            end += 1;
        }
        end
    }

    fn index_of(&self, needle: u8, from: usize) -> Option<usize> {
        if from > self.length {
            return None;
        }
        self.bytes[from..]
            .iter()
            .position(|&b| b == needle)
            .map(|i| i + from)
    }

    fn index_of_str(&self, needle: &str, from: usize) -> Option<usize> {
        if from > self.length {
            return None;
        }
        self.css[from..].find(needle).map(|i| i + from)
    }

    /// `RE_AT_END`: `[\t\n\f\r "#'()/;[\\\]{}]`
    fn find_at_end(&self, from: usize) -> Option<usize> {
        if from >= self.length {
            return None;
        }
        self.bytes[from..]
            .iter()
            .position(|&b| {
                matches!(
                    b,
                    TAB | NEWLINE
                        | FEED
                        | CR
                        | SPACE
                        | DOUBLE_QUOTE
                        | b'#'
                        | SINGLE_QUOTE
                        | OPEN_PARENTHESES
                        | CLOSE_PARENTHESES
                        | SLASH
                        | SEMICOLON
                        | OPEN_SQUARE
                        | BACKSLASH
                        | CLOSE_SQUARE
                        | OPEN_CURLY
                        | CLOSE_CURLY
                )
            })
            .map(|i| i + from)
    }

    /// `RE_WORD_END`: `[\t\n\f\r !"#'():;@[\\\]{}]|\/(?=\*)`
    fn find_word_end(&self, from: usize) -> Option<usize> {
        let mut i = from;
        while i < self.length {
            let b = self.bytes[i];
            let matched = matches!(
                b,
                TAB | NEWLINE
                    | FEED
                    | CR
                    | SPACE
                    | b'!'
                    | DOUBLE_QUOTE
                    | b'#'
                    | SINGLE_QUOTE
                    | OPEN_PARENTHESES
                    | CLOSE_PARENTHESES
                    | COLON
                    | SEMICOLON
                    | AT
                    | OPEN_SQUARE
                    | BACKSLASH
                    | CLOSE_SQUARE
                    | OPEN_CURLY
                    | CLOSE_CURLY
            ) || (b == SLASH && self.at(i + 1) == ASTERISK);
            if matched {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn unclosed(&self, what: &str) -> CssSyntaxError {
        self.input
            .error_at_offset(format!("Unclosed {}", what), self.pos)
    }
}

/// `RE_HEX_ESCAPE`: `[\da-f]/i`
fn is_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f' | b'A'..=b'F')
}

/// `RE_BAD_BRACKET`: `.[\r\n"'(/\\]` — any character that is not a line
/// terminator, followed by one of the listed characters.
fn is_bad_bracket(content: &str) -> bool {
    let bytes = content.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        let current = bytes[i];
        if current == NEWLINE || current == CR {
            continue;
        }
        // A UTF-8 continuation byte is not a character start, so it can never
        // be the `.` of the pattern.
        if current & 0xc0 == 0x80 {
            continue;
        }
        if matches!(
            bytes[i + 1],
            CR | NEWLINE | DOUBLE_QUOTE | SINGLE_QUOTE | OPEN_PARENTHESES | SLASH | BACKSLASH
        ) {
            return true;
        }
    }
    false
}
