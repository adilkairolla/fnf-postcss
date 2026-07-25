//! `Input` — a CSS source together with everything needed to report positions
//! inside it.
//!
//! Port of `lib/input.js`.
//!
//! ## Position model
//!
//! `offset` is a **UTF-8 byte** offset; `line`/`column` count **characters**
//! (1-based). PostCSS in JS counts UTF-16 code units for both. The three agree
//! for ASCII, which is all of CSS syntax itself; they differ only inside
//! non-ASCII identifiers, strings and comments.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{CssSyntaxError, ErrorInput, Pos};
use crate::options::InputOptions;
use crate::previous_map::PreviousMap;

/// A location inside an [`Input`], given either way round.
#[derive(Clone, Copy, Debug)]
pub enum Loc {
    /// UTF-8 byte offset.
    Offset(usize),
    /// 1-based line and character column.
    LineCol {
        /// 1-based line.
        line: usize,
        /// 1-based character column.
        column: usize,
    },
}

/// A CSS file (or string) being processed.
#[derive(Clone, Debug)]
pub struct Input {
    css: String,
    /// True when a byte order mark was stripped from the input.
    pub has_bom: bool,
    /// The enclosing document for CSS embedded in another language.
    document: Option<String>,
    /// Absolute path of the file, when known.
    pub file: Option<String>,
    /// Placeholder id used when no file is known.
    pub id: Option<String>,
    /// Source map found for this input.
    pub map: Option<PreviousMap>,
    /// Byte offset of the start of every line.
    line_index: Vec<usize>,
    /// Running count of UTF-8 continuation bytes at every
    /// [`CONTINUATION_BLOCK`] boundary, or `None` when the CSS is all ASCII.
    ///
    /// Columns count characters, so converting an offset into a column means
    /// knowing how many characters precede it on its line. Scanning for that is
    /// O(line length), which is fine for hand-written CSS and quadratic for the
    /// single-line output of a minifier — the shape a build pipeline actually
    /// feeds us. This table makes it O(1).
    continuation_index: Option<Vec<u32>>,
}

/// Bytes covered by one entry of [`Input::continuation_index`]. Small enough
/// that the residual scan is negligible, large enough that the table costs
/// ~1.5% of the input size.
const CONTINUATION_BLOCK: usize = 256;

impl Input {
    /// Creates an input from CSS with default options.
    pub fn from_css(css: impl Into<String>) -> Self {
        Input::new(css, InputOptions::default())
    }

    /// Creates an input from CSS and options.
    pub fn new(css: impl Into<String>, opts: InputOptions) -> Self {
        let mut css = css.into();

        // U+FEFF as a character, or the bytes of a UTF-16 BOM that survived a
        // lossy decode.
        let has_bom = if css.starts_with('\u{feff}') {
            css.drain(..'\u{feff}'.len_utf8());
            true
        } else if css.starts_with('\u{fffe}') {
            css.drain(..'\u{fffe}'.len_utf8());
            true
        } else {
            false
        };

        let file = opts.from.as_ref().map(|from| {
            if is_url(from) || Path::new(from).is_absolute() {
                from.clone()
            } else {
                absolute_path(from)
            }
        });

        let mut input = Input {
            line_index: build_line_index(&css),
            continuation_index: build_continuation_index(&css),
            css,
            has_bom,
            document: opts.document.clone(),
            file,
            id: None,
            map: None,
        };

        let map = PreviousMap::new(&input.css, &opts);
        if let Some(map) = map {
            if map.text.is_some() {
                if input.file.is_none() {
                    if let Some(file) = map.consumer().and_then(|c| c.file.clone()) {
                        input.file = Some(map.resolve(&file));
                    }
                }
                input.map = Some(map);
            }
        }

        if input.file.is_none() {
            input.id = Some(format!("<input css {}>", random_id()));
        }
        if let Some(map) = &mut input.map {
            map.file = input.file.clone().or_else(|| input.id.clone());
        }

        input
    }

    /// Rebuilds an input from its JSON form, without re-running BOM detection
    /// or path resolution.
    pub fn from_json_parts(
        css: String,
        has_bom: bool,
        file: Option<String>,
        id: Option<String>,
    ) -> Self {
        Input {
            line_index: build_line_index(&css),
            continuation_index: build_continuation_index(&css),
            css,
            has_bom,
            document: None,
            file,
            id,
            map: None,
        }
    }

    /// The CSS being processed, without any BOM.
    pub fn css(&self) -> &str {
        &self.css
    }

    /// The enclosing document, or the CSS itself when there is none.
    pub fn document(&self) -> &str {
        self.document.as_deref().unwrap_or(&self.css)
    }

    /// `file` when known, otherwise the generated `id`.
    pub fn from(&self) -> &str {
        self.file
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or("<input css>")
    }

    /// Converts a byte offset into a 1-based line and character column.
    pub fn from_offset(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.css.len());
        let line_index = match self.line_index.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let line_start = self.line_index[line_index];
        let column = self.chars_between(line_start, offset) + 1;
        (line_index + 1, column)
    }

    /// Number of characters in `css[start..end]`, in constant time.
    fn chars_between(&self, start: usize, end: usize) -> usize {
        let bytes = end - start;
        let Some(index) = &self.continuation_index else {
            // All-ASCII input: one byte per character.
            return bytes;
        };
        bytes - (self.continuations_before(index, end) - self.continuations_before(index, start))
    }

    /// Continuation bytes in `css[..offset]`: the tabulated count up to the
    /// enclosing block, plus a scan of at most [`CONTINUATION_BLOCK`] bytes.
    fn continuations_before(&self, index: &[u32], offset: usize) -> usize {
        let block = offset / CONTINUATION_BLOCK;
        let tabulated = index[block] as usize;
        let rest = &self.css.as_bytes()[block * CONTINUATION_BLOCK..offset];
        tabulated + rest.iter().filter(|byte| is_continuation(**byte)).count()
    }

    /// Converts a 1-based line and character column into a byte offset.
    pub fn from_line_and_column(&self, line: usize, column: usize) -> usize {
        let Some(&line_start) = self.line_index.get(line.saturating_sub(1)) else {
            return self.css.len();
        };
        let characters = column.saturating_sub(1);
        if self.continuation_index.is_none() {
            // All-ASCII input: the column is already a byte offset.
            return (line_start + characters).min(self.css.len());
        }
        let rest = &self.css[line_start..];
        match rest.char_indices().nth(characters) {
            Some((index, _)) => line_start + index,
            None => line_start + rest.len(),
        }
    }

    /// Builds an error at a byte offset, without an end position.
    pub fn error_at_offset(&self, message: impl Into<String>, offset: usize) -> CssSyntaxError {
        self.build_error(message.into(), Loc::Offset(offset), None, None)
    }

    /// Builds an error at a line and column, without an end position.
    pub fn error_at_line_col(
        &self,
        message: impl Into<String>,
        line: usize,
        column: usize,
    ) -> CssSyntaxError {
        self.build_error(message.into(), Loc::LineCol { line, column }, None, None)
    }

    /// Builds an error covering a range.
    pub fn error_range(
        &self,
        message: impl Into<String>,
        start: Loc,
        end: Loc,
        plugin: Option<&str>,
    ) -> CssSyntaxError {
        self.build_error(
            message.into(),
            start,
            Some(end),
            plugin.map(|p| p.to_string()),
        )
    }

    fn resolve_loc(&self, loc: Loc) -> (usize, usize, usize) {
        match loc {
            Loc::Offset(offset) => {
                let (line, column) = self.from_offset(offset);
                (line, column, offset)
            }
            Loc::LineCol { line, column } => {
                (line, column, self.from_line_and_column(line, column))
            }
        }
    }

    fn build_error(
        &self,
        message: String,
        start: Loc,
        end: Option<Loc>,
        plugin: Option<String>,
    ) -> CssSyntaxError {
        let (line, column, offset) = self.resolve_loc(start);
        let end = end.map(|end| self.resolve_loc(end));

        let mut error = match self.origin(line, column, end.map(|(l, c, _)| (l, c))) {
            Some(origin) => CssSyntaxError::with_position(
                message,
                Some(Pos {
                    line: origin.line,
                    column: origin.column,
                }),
                match (origin.end_line, origin.end_column) {
                    (Some(line), Some(column)) => Some(Pos { line, column }),
                    _ => None,
                },
                origin.source,
                origin.file,
                plugin,
            ),
            None => CssSyntaxError::with_position(
                message,
                Some(Pos { line, column }),
                end.map(|(line, column, _)| Pos { line, column }),
                Some(self.css.clone()),
                self.file.clone(),
                plugin,
            ),
        };

        error.input = Some(ErrorInput {
            column,
            end_column: end.map(|(_, column, _)| column),
            end_line: end.map(|(line, _, _)| line),
            end_offset: end.map(|(_, _, offset)| offset),
            file: self.file.clone(),
            line,
            offset,
            source: self.css.clone(),
            url: self.file.as_deref().map(path_to_file_url),
        });

        error
    }

    /// Maps a generated position back through this input's source map.
    ///
    /// Port of `Input#origin()`.
    pub fn origin(
        &self,
        line: usize,
        column: usize,
        end: Option<(usize, usize)>,
    ) -> Option<Origin> {
        let map = self.map.as_ref()?;
        let consumer = map.consumer()?;

        let from = consumer.original_position_for(line, column.saturating_sub(1));
        let source = from.source?;
        let from_line = from.line?;
        let from_column = from.column?;

        let to = end.and_then(|(end_line, end_column)| {
            let position = consumer.original_position_for(end_line, end_column.saturating_sub(1));
            // A map need not cover the end position; treat a miss as if no end
            // had been requested, so the pair stays consistent.
            position.source.is_some().then_some(position)
        });

        let url = if Path::new(&source).is_absolute() {
            path_to_file_url(&source)
        } else {
            let base = consumer
                .source_root
                .clone()
                .or_else(|| map.file.as_deref().map(path_to_file_url))
                .unwrap_or_default();
            resolve_url(&base, &source)
        };

        let file = file_url_to_path(&url);

        Some(Origin {
            column: from_column + 1,
            end_column: to.as_ref().and_then(|to| to.column).map(|c| c + 1),
            end_line: to.as_ref().and_then(|to| to.line),
            line: from_line,
            file,
            source: consumer.source_content_for(&source).map(|s| s.to_string()),
            url,
        })
    }
}

/// A position resolved through a previous source map.
#[derive(Clone, Debug)]
pub struct Origin {
    /// 1-based character column in the original file.
    pub column: usize,
    /// 1-based character column of the range end.
    pub end_column: Option<usize>,
    /// 1-based line of the range end.
    pub end_line: Option<usize>,
    /// 1-based line in the original file.
    pub line: usize,
    /// Filesystem path, when the resolved URL is a `file:` URL.
    pub file: Option<String>,
    /// Original source text, when the map embeds it.
    pub source: Option<String>,
    /// URL of the original file, resolved against the map's location.
    pub url: String,
}

fn build_line_index(css: &str) -> Vec<usize> {
    let mut index = vec![0];
    for (offset, byte) in css.bytes().enumerate() {
        if byte == b'\n' {
            index.push(offset + 1);
        }
    }
    index
}

/// Counts UTF-8 continuation bytes — bytes that continue a character rather
/// than starting one — so `characters = bytes - continuations`.
fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// Builds the continuation table, or `None` for all-ASCII input, where every
/// byte is a character and the table would be all zeroes.
fn build_continuation_index(css: &str) -> Option<Vec<u32>> {
    if css.is_ascii() {
        return None;
    }
    let mut index = Vec::with_capacity(css.len() / CONTINUATION_BLOCK + 1);
    let mut total = 0u32;
    for (block, bytes) in css.as_bytes().chunks(CONTINUATION_BLOCK).enumerate() {
        debug_assert_eq!(index.len(), block);
        index.push(total);
        total += bytes.iter().filter(|byte| is_continuation(**byte)).count() as u32;
    }
    index.push(total);
    Some(index)
}

pub(crate) fn is_url(path: &str) -> bool {
    match path.find("://") {
        Some(index) => index > 0 && path[..index].chars().all(|c| c.is_ascii_alphanumeric()),
        None => false,
    }
}

pub(crate) fn absolute_path(path: &str) -> String {
    let path = Path::new(path);
    if path.is_absolute() {
        return normalize(path);
    }
    match std::env::current_dir() {
        Ok(cwd) => normalize(&cwd.join(path)),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Collapses `.` and `..` without touching the filesystem, like
/// `path.resolve()`.
fn normalize(path: &Path) -> String {
    use std::path::Component;
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().into_owned()
}

/// `pathToFileURL()`: percent-encodes the characters Node encodes.
pub(crate) fn path_to_file_url(path: &str) -> String {
    let mut url = String::from("file://");
    for byte in path.bytes() {
        match byte {
            b'/' | b'-' | b'_' | b'.' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*'
            | b'+' | b',' | b'=' | b'@' | b':' => url.push(byte as char),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => url.push(byte as char),
            other => url.push_str(&format!("%{:02X}", other)),
        }
    }
    url
}

/// Inverse of [`path_to_file_url`], returning `None` for other schemes.
pub(crate) fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let mut path = String::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                path.push(byte as char);
                i += 3;
                continue;
            }
        }
        path.push(bytes[i] as char);
        i += 1;
    }
    Some(path)
}

/// Resolves `source` against `base`, which may be a URL or a path.
fn resolve_url(base: &str, source: &str) -> String {
    if is_url(source) {
        return source.to_string();
    }
    if base.is_empty() {
        return source.to_string();
    }
    let base_dir = match base.rfind('/') {
        Some(index) => &base[..index],
        None => "",
    };
    crate::source_map::join_path(base_dir, source)
}

/// A tiny xorshift generator, standing in for `nanoid(6)`.
fn random_id() -> String {
    const ALPHABET: &[u8] = b"useandom26T198340PX75pxJACKVERYMINDBUSHWOLFGQZbfghjklqvwyzrict";
    static STATE: AtomicU64 = AtomicU64::new(0);

    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545_F491_4F6C_DD1D)
            | 1;
    }

    let mut id = String::with_capacity(6);
    for _ in 0..6 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        id.push(ALPHABET[(state % ALPHABET.len() as u64) as usize] as char);
    }
    STATE.store(state, Ordering::Relaxed);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_offsets_to_line_and_column() {
        let input = Input::from_css("a {\n  color: red;\n}\n");
        assert_eq!(input.from_offset(0), (1, 1));
        assert_eq!(input.from_offset(2), (1, 3));
        assert_eq!(input.from_offset(4), (2, 1));
        assert_eq!(input.from_offset(6), (2, 3));
        assert_eq!(input.from_offset(18), (3, 1));

        for offset in 0..input.css().len() {
            let (line, column) = input.from_offset(offset);
            assert_eq!(input.from_line_and_column(line, column), offset);
        }
    }

    #[test]
    fn counts_columns_in_characters() {
        let input = Input::from_css("/* é */\na{}");
        // The comment is 9 bytes but 8 characters.
        assert_eq!(input.from_offset(9), (2, 1));
        assert_eq!(input.from_offset(6), (1, 6));
    }

    /// The constant-time column lookup must agree with a naive scan everywhere,
    /// including across the block boundaries of the continuation table and on a
    /// single very long line — the shape a minifier emits, which is what made
    /// the naive version quadratic.
    #[test]
    fn matches_a_naive_scan_on_a_long_line_with_multibyte_characters() {
        // Multi-byte characters of 2, 3 and 4 bytes, spaced so that characters
        // straddle CONTINUATION_BLOCK boundaries rather than sitting inside one.
        let mut css = String::new();
        for index in 0..2000 {
            css.push_str("a{content:\"");
            match index % 3 {
                0 => css.push('é'), // 2 bytes
                1 => css.push('☃'), // 3 bytes
                _ => css.push('𝄞'), // 4 bytes
            }
            css.push_str("\"}");
        }
        assert!(css.len() > 20 * CONTINUATION_BLOCK);

        let input = Input::from_css(css.clone());
        assert!(input.continuation_index.is_some(), "table should be built");

        for (offset, _) in css.char_indices() {
            let naive = css[..offset].chars().count() + 1;
            assert_eq!(
                input.from_offset(offset),
                (1, naive),
                "offset {offset} on a {} byte single line",
                css.len()
            );
            assert_eq!(input.from_line_and_column(1, naive), offset);
        }
        // One past the end, where a node's `end` position can land.
        assert_eq!(input.from_offset(css.len()).1, css.chars().count() + 1);
    }

    /// The all-ASCII fast path has to answer exactly what the scan would.
    #[test]
    fn ascii_fast_path_matches_a_naive_scan() {
        let css = format!("{}\n{}", "a{color:red}".repeat(500), "b{top:0}".repeat(500));
        let input = Input::from_css(css.clone());
        assert!(input.continuation_index.is_none(), "no table for ASCII");

        for offset in 0..=css.len() {
            let (line, column) = input.from_offset(offset);
            let line_start = css[..offset].rfind('\n').map_or(0, |index| index + 1);
            assert_eq!(column, offset - line_start + 1, "column at {offset}");
            assert_eq!(input.from_line_and_column(line, column), offset);
        }
    }

    #[test]
    fn strips_bom() {
        let input = Input::from_css("\u{feff}a{}");
        assert!(input.has_bom);
        assert_eq!(input.css(), "a{}");

        let input = Input::from_css("a{}");
        assert!(!input.has_bom);
    }

    #[test]
    fn generates_an_id_without_a_file() {
        let input = Input::from_css("a{}");
        assert!(input.file.is_none());
        assert!(input.from().starts_with("<input css "));
        assert_eq!(input.from().len(), "<input css ".len() + 7);
    }

    #[test]
    fn converts_paths_to_file_urls() {
        assert_eq!(path_to_file_url("/a/b.css"), "file:///a/b.css");
        assert_eq!(path_to_file_url("/a b.css"), "file:///a%20b.css");
        assert_eq!(
            file_url_to_path("file:///a%20b.css").as_deref(),
            Some("/a b.css")
        );
        assert_eq!(file_url_to_path("https://a/b.css"), None);
    }
}
