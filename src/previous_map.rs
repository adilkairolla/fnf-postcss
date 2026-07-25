//! `PreviousMap` — finds and loads the source map of the CSS being parsed, so a
//! new map can be chained onto it.
//!
//! Port of `lib/previous-map.js`.

use std::path::{Component, Path, PathBuf};

use crate::options::{InputOptions, MapSetting, PrevMap};
use crate::source_map::SourceMapConsumer;
use crate::vlq::base64_decode;

/// A source map belonging to the input CSS.
#[derive(Clone, Debug)]
pub struct PreviousMap {
    /// The `sourceMappingURL` value found in the CSS.
    pub annotation: Option<String>,
    /// True when the annotation is a `data:` URI.
    pub inline: bool,
    /// Raw map JSON.
    pub text: Option<String>,
    /// Path of the map file, when it was read from disk.
    pub map_file: Option<String>,
    /// Directory the map's `sources` are relative to.
    pub root: Option<String>,
    /// Path of the CSS file the map belongs to.
    pub file: Option<String>,
    /// Set when [`PreviousMap::text`] could not be parsed.
    pub parse_error: Option<String>,
    consumer: Option<SourceMapConsumer>,
}

impl PreviousMap {
    /// Looks for a map for `css`, returning `None` when maps are disabled.
    pub fn new(css: &str, opts: &InputOptions) -> Option<Self> {
        if matches!(opts.map, Some(MapSetting::Disabled)) {
            return None;
        }

        let mut map = PreviousMap {
            annotation: load_annotation(css),
            inline: false,
            text: None,
            map_file: None,
            root: None,
            file: None,
            parse_error: None,
            consumer: None,
        };
        map.inline = map
            .annotation
            .as_deref()
            .is_some_and(|annotation| annotation.starts_with("data:"));

        let prev = opts
            .map
            .as_ref()
            .map(|setting| setting.options())
            .and_then(|options| options.prev);

        let text = map.load_map(opts.from.as_deref(), prev.as_ref(), opts.unsafe_map);

        if map.map_file.is_none() {
            if let Some(from) = &opts.from {
                map.map_file = Some(from.clone());
            }
        }
        if let Some(map_file) = &map.map_file {
            map.root = dirname(map_file);
        }
        if let Some(text) = text {
            match SourceMapConsumer::from_json(&text) {
                Ok(consumer) => map.consumer = Some(consumer),
                Err(error) => map.parse_error = Some(error),
            }
            map.text = Some(text);
        }

        Some(map)
    }

    /// The parsed map, if one was found and is valid.
    pub fn consumer(&self) -> Option<&SourceMapConsumer> {
        self.consumer.as_ref()
    }

    /// True when the map embeds its sources.
    pub fn with_content(&self) -> bool {
        self.consumer
            .as_ref()
            .is_some_and(|consumer| consumer.has_contents())
    }

    /// Resolves a path found inside the map against the map's own location.
    pub fn resolve(&self, file: &str) -> String {
        if crate::input::is_url(file) {
            return file.to_string();
        }
        let root = self
            .consumer
            .as_ref()
            .and_then(|consumer| consumer.source_root.clone())
            .or_else(|| self.root.clone())
            .unwrap_or_else(|| ".".to_string());
        crate::input::absolute_path(&join(&root, file))
    }

    fn load_map(
        &mut self,
        file: Option<&str>,
        prev: Option<&PrevMap>,
        unsafe_map: bool,
    ) -> Option<String> {
        match prev {
            Some(PrevMap::Disabled) => None,
            Some(PrevMap::Text(text)) => Some(text.clone()),
            Some(PrevMap::Raw(raw)) => serde_json::to_string(raw.as_ref()).ok(),
            Some(PrevMap::File(path)) => {
                let path = path.to_string_lossy().into_owned();
                match self.load_file(&path, file, true, unsafe_map) {
                    Some(text) => Some(text),
                    None => {
                        self.parse_error =
                            Some(format!("Unable to load previous source map: {}", path));
                        None
                    }
                }
            }
            None => {
                if self.inline {
                    let annotation = self.annotation.clone()?;
                    match decode_inline(&annotation) {
                        Ok(text) => Some(text),
                        Err(error) => {
                            self.parse_error = Some(error);
                            None
                        }
                    }
                } else if let Some(annotation) = self.annotation.clone() {
                    let path = match file.and_then(dirname) {
                        Some(dir) => join(&dir, &annotation),
                        None => annotation,
                    };
                    self.load_file(&path, file, false, unsafe_map)
                } else {
                    None
                }
            }
        }
    }

    /// Reads a map file, refusing paths a strict resolver would reject unless
    /// the read was explicitly requested or `unsafe_map` is set.
    fn load_file(
        &mut self,
        path: &str,
        css_file: Option<&str>,
        trusted: bool,
        unsafe_map: bool,
    ) -> Option<String> {
        if !trusted && !unsafe_map {
            if !path.to_lowercase().ends_with(".map") {
                return None;
            }
            let css_file = css_file?;
            let base = dirname(css_file)?;
            let rel = relative(&base, path);
            if rel == ".." || rel.starts_with("../") || Path::new(&rel).is_absolute() {
                return None;
            }
        }

        self.root = dirname(path);
        let contents = std::fs::read_to_string(path).ok()?;
        self.map_file = Some(path.to_string());
        // A map may be served with an XSSI prefix.
        let contents = match contents.find('\n') {
            Some(index) if contents.starts_with(")]}'") => contents[index + 1..].to_string(),
            _ => contents,
        };
        Some(contents.trim().to_string())
    }
}

/// Finds the last `sourceMappingURL` annotation in the CSS.
fn load_annotation(css: &str) -> Option<String> {
    const NEEDLE: &str = "# sourceMappingURL=";
    let mut start = None;

    // `/\/\*\s*# sourceMappingURL=/g`, keeping the last match.
    let mut search = 0;
    while let Some(found) = css[search..].find("/*") {
        let comment_start = search + found;
        let after = &css[comment_start + 2..];
        let trimmed = after.trim_start_matches(|c: char| c.is_whitespace());
        if trimmed.starts_with(NEEDLE) {
            start = Some(comment_start);
        }
        search = comment_start + 2;
    }

    let start = start?;
    let end = css[start..].find("*/")? + start;
    let body = &css[start..end];
    let url = body
        .trim_start_matches("/*")
        .trim_start()
        .trim_start_matches(NEEDLE)
        .trim();
    Some(url.to_string())
}

/// Decodes a `data:application/json` source map URI.
fn decode_inline(text: &str) -> Result<String, String> {
    const BASE_CHARSET_URI: [&str; 2] = [
        "data:application/json;charset=utf8;base64,",
        "data:application/json;charset=utf-8;base64,",
    ];
    const BASE_URI: &str = "data:application/json;base64,";
    const CHARSET_URI: [&str; 2] = [
        "data:application/json;charset=utf8,",
        "data:application/json;charset=utf-8,",
    ];
    const URI: &str = "data:application/json,";

    for prefix in CHARSET_URI.iter().chain(std::iter::once(&URI)) {
        if let Some(rest) = text.strip_prefix(*prefix) {
            return Ok(decode_uri_component(rest));
        }
    }
    for prefix in BASE_CHARSET_URI.iter().chain(std::iter::once(&BASE_URI)) {
        if let Some(rest) = text.strip_prefix(*prefix) {
            let bytes = base64_decode(rest)
                .ok_or_else(|| "Invalid base64 in inline source map".to_string())?;
            return String::from_utf8(bytes)
                .map_err(|_| "Inline source map is not valid UTF-8".to_string());
        }
    }

    let encoding = text
        .strip_prefix("data:application/json;")
        .and_then(|rest| rest.split(',').next())
        .unwrap_or("");
    Err(format!("Unsupported source map encoding {}", encoding))
}

fn decode_uri_component(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn dirname(path: &str) -> Option<String> {
    let parent = Path::new(path).parent()?;
    let parent = parent.to_string_lossy();
    Some(if parent.is_empty() {
        ".".to_string()
    } else {
        parent.into_owned()
    })
}

pub(crate) fn join(base: &str, path: &str) -> String {
    if Path::new(path).is_absolute() {
        return path.to_string();
    }
    let joined = Path::new(base).join(path);
    let mut result = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().into_owned()
}

/// `path.relative()`.
pub(crate) fn relative(base: &str, target: &str) -> String {
    let base = crate::input::absolute_path(base);
    let target = crate::input::absolute_path(target);
    let base = PathBuf::from(base);
    let target = PathBuf::from(target);

    let mut base_parts = base.components().peekable();
    let mut target_parts = target.components().peekable();

    while base_parts.peek().is_some() && base_parts.peek() == target_parts.peek() {
        base_parts.next();
        target_parts.next();
    }

    let mut result = PathBuf::new();
    for _ in base_parts {
        result.push("..");
    }
    for part in target_parts {
        result.push(part.as_os_str());
    }
    result.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_last_annotation() {
        let css = "a{}\n/*# sourceMappingURL=a.css.map */";
        assert_eq!(load_annotation(css).as_deref(), Some("a.css.map"));

        let css = "a{}\n/*# sourceMappingURL=a.map */\nb{}\n/*# sourceMappingURL=b.map */";
        assert_eq!(load_annotation(css).as_deref(), Some("b.map"));

        assert_eq!(load_annotation("a{}"), None);
    }

    #[test]
    fn accepts_whitespace_before_the_hash() {
        let css = "/*  # sourceMappingURL=a.map */";
        assert_eq!(load_annotation(css).as_deref(), Some("a.map"));
    }

    #[test]
    fn decodes_inline_maps() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let encoded = crate::vlq::base64_encode(json.as_bytes());
        let uri = format!("data:application/json;base64,{}", encoded);
        assert_eq!(decode_inline(&uri).unwrap(), json);

        let uri = format!("data:application/json,{}", json.replace(',', "%2C"));
        assert_eq!(decode_inline(&uri).unwrap(), json);

        assert!(decode_inline("data:application/json;gzip,x")
            .unwrap_err()
            .contains("Unsupported source map encoding gzip"));
    }

    #[test]
    fn computes_relative_paths() {
        assert_eq!(relative("/a/b", "/a/b/c.css"), "c.css");
        assert_eq!(relative("/a/b", "/a/c.css"), "../c.css");
    }
}
