//! Source map reading and writing.
//!
//! Replaces the `source-map-js` dependency: [`SourceMapConsumer`] covers the
//! parts of the consumer API PostCSS uses (`originalPositionFor`,
//! `sourceContentFor`, `sources`, `file`, `sourceRoot`) and
//! [`SourceMapGenerator`] covers `addMapping`, `setSourceContent`,
//! `applySourceMap` and serialization.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::vlq::{decode_vlq, encode_vlq};

/// The raw JSON shape of a v3 source map.
///
/// Field order matches `SourceMapGenerator#toJSON()` in `source-map-js`, so a
/// serialized map — and therefore an inline `data:` annotation — is byte-identical
/// to the JS implementation's.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RawSourceMap {
    /// Always 3 for maps this crate writes.
    pub version: Option<u32>,
    /// The original files, as URLs or paths.
    pub sources: Vec<String>,
    /// Identifier names, unused by CSS maps.
    #[serde(default)]
    pub names: Vec<String>,
    /// The base64 VLQ encoded mappings.
    pub mappings: String,
    /// The generated file this map describes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Prefix prepended to every entry of `sources`.
    #[serde(rename = "sourceRoot", skip_serializing_if = "Option::is_none")]
    pub source_root: Option<String>,
    /// The text of each source, when embedded.
    #[serde(rename = "sourcesContent", skip_serializing_if = "Option::is_none")]
    pub sources_content: Option<Vec<Option<String>>>,
}

/// One decoded mapping segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mapping {
    /// 1-based line in the generated file.
    pub generated_line: usize,
    /// 0-based column in the generated file.
    pub generated_column: usize,
    /// Index into `sources`, when the segment has an original position.
    pub source: Option<usize>,
    /// 1-based line in the original file.
    pub original_line: Option<usize>,
    /// 0-based column in the original file.
    pub original_column: Option<usize>,
    /// Index into `names`.
    pub name: Option<usize>,
}

/// A resolved original position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginalPosition {
    /// The original file.
    pub source: Option<String>,
    /// 1-based line in the original file.
    pub line: Option<usize>,
    /// 0-based column in the original file.
    pub column: Option<usize>,
    /// Identifier name, if the map records one.
    pub name: Option<String>,
}

/// Reads positions out of an existing source map.
#[derive(Clone, Debug)]
pub struct SourceMapConsumer {
    /// The generated file the map describes.
    pub file: Option<String>,
    /// Prefix prepended to every source.
    pub source_root: Option<String>,
    /// Sources as written in the map, before `sourceRoot` is applied.
    pub raw_sources: Vec<String>,
    /// Identifier names.
    pub names: Vec<String>,
    /// The text of each source, when embedded.
    pub sources_content: Option<Vec<Option<String>>>,
    mappings: Vec<Mapping>,
}

impl SourceMapConsumer {
    /// Parses a source map from JSON text.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let raw: RawSourceMap =
            serde_json::from_str(text).map_err(|e| format!("Invalid source map: {}", e))?;
        Ok(Self::from_raw(raw))
    }

    /// Builds a consumer from an already parsed map.
    pub fn from_raw(raw: RawSourceMap) -> Self {
        let mut mappings = decode_mappings(&raw.mappings);
        // Lookups binary-search by generated position.
        mappings.sort_by_key(|m| (m.generated_line, m.generated_column));
        SourceMapConsumer {
            file: raw.file,
            source_root: raw.source_root,
            raw_sources: raw.sources,
            names: raw.names,
            sources_content: raw.sources_content,
            mappings,
        }
    }

    /// Sources with `sourceRoot` applied, as `consumer.sources` returns them.
    pub fn sources(&self) -> Vec<String> {
        self.raw_sources
            .iter()
            .map(|source| compute_source_url(self.source_root.as_deref(), source))
            .collect()
    }

    /// All decoded mappings, ordered by generated position.
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    /// Original position for a generated position (1-based line, 0-based
    /// column), using a greatest-lower-bound search within the same line.
    pub fn original_position_for(&self, line: usize, column: usize) -> OriginalPosition {
        let index = self
            .mappings
            .partition_point(|m| (m.generated_line, m.generated_column) <= (line, column));

        if index == 0 {
            return OriginalPosition::default();
        }
        let mapping = &self.mappings[index - 1];
        if mapping.generated_line != line {
            return OriginalPosition::default();
        }

        OriginalPosition {
            source: mapping
                .source
                .and_then(|i| self.raw_sources.get(i))
                .map(|source| compute_source_url(self.source_root.as_deref(), source)),
            line: mapping.original_line,
            column: mapping.original_column,
            name: mapping
                .name
                .and_then(|i| self.names.get(i))
                .map(|name| name.to_string()),
        }
    }

    /// Content of an original source, if the map embeds it.
    ///
    /// Accepts the source with or without `sourceRoot` applied.
    pub fn source_content_for(&self, source: &str) -> Option<&str> {
        let contents = self.sources_content.as_ref()?;
        let relative = self
            .source_root
            .as_deref()
            .and_then(|root| strip_prefix_path(source, root));

        for (index, raw) in self.raw_sources.iter().enumerate() {
            let matched = raw == source
                || Some(raw.as_str()) == relative
                || compute_source_url(self.source_root.as_deref(), raw) == source;
            if matched {
                return contents.get(index).and_then(|c| c.as_deref());
            }
        }
        None
    }

    /// True when the map carries at least one embedded source.
    pub fn has_contents(&self) -> bool {
        self.sources_content
            .as_ref()
            .is_some_and(|contents| !contents.is_empty())
    }

    /// Rebuilds a generator holding the same mappings, like
    /// `SourceMapGenerator.fromSourceMap()`.
    pub fn to_generator(&self) -> SourceMapGenerator {
        let mut generator = SourceMapGenerator::new(self.file.clone());
        generator.source_root = self.source_root.clone();

        for mapping in &self.mappings {
            let source = mapping
                .source
                .and_then(|i| self.raw_sources.get(i))
                .map(|s| compute_source_url(self.source_root.as_deref(), s));
            generator.add_mapping(GeneratedMapping {
                generated_line: mapping.generated_line,
                generated_column: mapping.generated_column,
                original_line: mapping.original_line,
                original_column: mapping.original_column,
                source,
                name: mapping.name.and_then(|i| self.names.get(i)).cloned(),
            });
        }

        if let Some(contents) = &self.sources_content {
            for (index, content) in contents.iter().enumerate() {
                if let (Some(content), Some(source)) = (content, self.raw_sources.get(index)) {
                    let source = compute_source_url(self.source_root.as_deref(), source);
                    generator.set_source_content(&source, Some(content.clone()));
                }
            }
        }

        generator
    }
}

/// A mapping as handed to [`SourceMapGenerator::add_mapping`].
#[derive(Clone, Debug, Default)]
pub struct GeneratedMapping {
    /// 1-based line in the generated file.
    pub generated_line: usize,
    /// 0-based column in the generated file.
    pub generated_column: usize,
    /// 1-based line in the original file.
    pub original_line: Option<usize>,
    /// 0-based column in the original file.
    pub original_column: Option<usize>,
    /// The original file.
    pub source: Option<String>,
    /// Identifier name.
    pub name: Option<String>,
}

/// Builds a source map from mappings added in generated order.
#[derive(Clone, Debug, Default)]
pub struct SourceMapGenerator {
    /// The generated file this map will describe.
    pub file: Option<String>,
    /// Prefix prepended to every source.
    pub source_root: Option<String>,
    mappings: Vec<GeneratedMapping>,
    sources: Vec<String>,
    names: Vec<String>,
    contents: HashMap<String, String>,
}

impl SourceMapGenerator {
    /// An empty map for `file`.
    pub fn new(file: Option<String>) -> Self {
        SourceMapGenerator {
            file,
            source_root: None,
            mappings: Vec::new(),
            sources: Vec::new(),
            names: Vec::new(),
            contents: HashMap::new(),
        }
    }

    /// Records one mapping.
    pub fn add_mapping(&mut self, mapping: GeneratedMapping) {
        if let Some(source) = &mapping.source {
            if !self.sources.iter().any(|s| s == source) {
                self.sources.push(source.clone());
            }
        }
        if let Some(name) = &mapping.name {
            if !self.names.iter().any(|n| n == name) {
                self.names.push(name.clone());
            }
        }
        self.mappings.push(mapping);
    }

    /// Embeds (or drops) the text of one source.
    pub fn set_source_content(&mut self, source: &str, content: Option<String>) {
        let key = match &self.source_root {
            Some(root) => relative_path(root, source),
            None => source.to_string(),
        };
        match content {
            Some(content) => {
                self.contents.insert(key, content);
            }
            None => {
                self.contents.remove(&key);
            }
        }
    }

    /// Retargets this map's mappings through `consumer`, so the result points at
    /// the sources `consumer` was built from.
    ///
    /// Port of `SourceMapGenerator#applySourceMap`.
    pub fn apply_source_map(
        &mut self,
        consumer: &SourceMapConsumer,
        source_file: Option<&str>,
        source_map_path: Option<&str>,
    ) {
        let source_file = match source_file.map(|f| f.to_string()).or(consumer.file.clone()) {
            Some(file) => file,
            // Nothing identifies which of our sources the map replaces.
            None => return,
        };
        let source_file = match &self.source_root {
            Some(root) => relative_path(root, &source_file),
            None => source_file,
        };

        let mut new_sources: Vec<String> = Vec::new();
        let mut new_names: Vec<String> = Vec::new();

        for mapping in &mut self.mappings {
            let replaces = mapping.source.as_deref() == Some(source_file.as_str())
                && mapping.original_line.is_some();
            if replaces {
                let original = consumer.original_position_for(
                    mapping.original_line.unwrap_or(0),
                    mapping.original_column.unwrap_or(0),
                );
                if let Some(source) = original.source {
                    let mut source = source;
                    if let Some(path) = source_map_path {
                        source = join_path(path, &source);
                    }
                    if let Some(root) = &self.source_root {
                        source = relative_path(root, &source);
                    }
                    mapping.source = Some(source);
                    mapping.original_line = original.line;
                    mapping.original_column = original.column;
                    if original.name.is_some() {
                        mapping.name = original.name;
                    }
                }
            }

            if let Some(source) = &mapping.source {
                if !new_sources.iter().any(|s| s == source) {
                    new_sources.push(source.clone());
                }
            }
            if let Some(name) = &mapping.name {
                if !new_names.iter().any(|n| n == name) {
                    new_names.push(name.clone());
                }
            }
        }

        self.sources = new_sources;
        self.names = new_names;

        for source in consumer.sources() {
            if let Some(content) = consumer.source_content_for(&source) {
                let mut target = source.clone();
                if let Some(path) = source_map_path {
                    target = join_path(path, &target);
                }
                if let Some(root) = &self.source_root {
                    target = relative_path(root, &target);
                }
                let content = content.to_string();
                self.set_source_content(&target, Some(content));
            }
        }
    }

    /// Drops every embedded source, for `map.sourcesContent: false`.
    pub fn clear_source_contents(&mut self) {
        self.contents.clear();
    }

    /// Serializes to the v3 JSON model.
    pub fn to_raw(&self) -> RawSourceMap {
        let sources_content = if self.contents.is_empty() {
            None
        } else {
            Some(
                self.sources
                    .iter()
                    .map(|source| {
                        let key = match &self.source_root {
                            Some(root) => relative_path(root, source),
                            None => source.clone(),
                        };
                        self.contents.get(&key).cloned()
                    })
                    .collect(),
            )
        };

        RawSourceMap {
            version: Some(3),
            sources: self.sources.clone(),
            names: self.names.clone(),
            mappings: self.encode_mappings(),
            file: self.file.clone(),
            source_root: self.source_root.clone(),
            sources_content,
        }
    }

    /// Serializes to JSON text.
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&self.to_raw()).expect("source map is serializable")
    }

    fn encode_mappings(&self) -> String {
        let mut mappings: Vec<&GeneratedMapping> = self.mappings.iter().collect();
        mappings.sort_by_key(|m| (m.generated_line, m.generated_column));

        let mut out = String::new();
        let mut previous_generated_line = 1;
        let mut previous_generated_column = 0i64;
        let mut previous_original_line = 0i64;
        let mut previous_original_column = 0i64;
        let mut previous_source = 0i64;
        let mut previous_name = 0i64;

        for (index, mapping) in mappings.iter().enumerate() {
            if mapping.generated_line != previous_generated_line {
                previous_generated_column = 0;
                while mapping.generated_line != previous_generated_line {
                    out.push(';');
                    previous_generated_line += 1;
                }
            } else if index > 0 {
                if same_position(mapping, mappings[index - 1]) {
                    continue;
                }
                out.push(',');
            }

            encode_vlq(
                mapping.generated_column as i64 - previous_generated_column,
                &mut out,
            );
            previous_generated_column = mapping.generated_column as i64;

            if let Some(source) = &mapping.source {
                let source_index =
                    self.sources
                        .iter()
                        .position(|s| s == source)
                        .expect("source was registered by add_mapping") as i64;
                encode_vlq(source_index - previous_source, &mut out);
                previous_source = source_index;

                // Original line numbers are 1-based in the model and 0-based in
                // the encoding.
                let original_line = mapping.original_line.unwrap_or(1) as i64 - 1;
                encode_vlq(original_line - previous_original_line, &mut out);
                previous_original_line = original_line;

                let original_column = mapping.original_column.unwrap_or(0) as i64;
                encode_vlq(original_column - previous_original_column, &mut out);
                previous_original_column = original_column;

                if let Some(name) = &mapping.name {
                    let name_index = self
                        .names
                        .iter()
                        .position(|n| n == name)
                        .expect("name was registered by add_mapping")
                        as i64;
                    encode_vlq(name_index - previous_name, &mut out);
                    previous_name = name_index;
                }
            }
        }

        out
    }
}

fn same_position(a: &GeneratedMapping, b: &GeneratedMapping) -> bool {
    a.generated_line == b.generated_line
        && a.generated_column == b.generated_column
        && a.source == b.source
        && a.original_line == b.original_line
        && a.original_column == b.original_column
        && a.name == b.name
}

/// Decodes the `mappings` field into flat segments.
pub fn decode_mappings(mappings: &str) -> Vec<Mapping> {
    let mut result = Vec::new();
    let mut generated_line = 1;
    // Reset at the start of every line group.
    let mut previous_generated_column;
    let mut previous_source = 0i64;
    let mut previous_original_line = 0i64;
    let mut previous_original_column = 0i64;
    let mut previous_name = 0i64;

    for segment_group in mappings.split(';') {
        previous_generated_column = 0i64;
        for segment in segment_group.split(',') {
            if segment.is_empty() {
                continue;
            }
            let bytes = segment.as_bytes();
            let mut offset = 0;

            let Some((generated_column, read)) = decode_vlq(&bytes[offset..]) else {
                break;
            };
            offset += read;
            previous_generated_column += generated_column;

            let mut mapping = Mapping {
                generated_line,
                generated_column: previous_generated_column.max(0) as usize,
                ..Mapping::default()
            };

            if offset < bytes.len() {
                if let Some((source, read)) = decode_vlq(&bytes[offset..]) {
                    offset += read;
                    previous_source += source;
                    mapping.source = Some(previous_source.max(0) as usize);

                    if let Some((line, read)) = decode_vlq(&bytes[offset..]) {
                        offset += read;
                        previous_original_line += line;
                        // 0-based in the encoding, 1-based in the model.
                        mapping.original_line = Some((previous_original_line.max(0) + 1) as usize);

                        if let Some((column, read)) = decode_vlq(&bytes[offset..]) {
                            offset += read;
                            previous_original_column += column;
                            mapping.original_column =
                                Some(previous_original_column.max(0) as usize);

                            if offset < bytes.len() {
                                if let Some((name, _)) = decode_vlq(&bytes[offset..]) {
                                    previous_name += name;
                                    mapping.name = Some(previous_name.max(0) as usize);
                                }
                            }
                        }
                    }
                }
            }

            result.push(mapping);
        }
        generated_line += 1;
    }

    result
}

/// `util.computeSourceURL()` without a map URL: prefixes `sourceRoot` unless
/// the source is already absolute.
pub fn compute_source_url(source_root: Option<&str>, source: &str) -> String {
    match source_root {
        Some(root) if !root.is_empty() && !is_absolute_url(source) && !source.starts_with('/') => {
            join_path(root, source)
        }
        _ => source.to_string(),
    }
}

fn is_absolute_url(path: &str) -> bool {
    match path.find("://") {
        Some(index) => path[..index].chars().all(|c| c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// `util.join()`: joins with `/`, letting an absolute path win.
pub fn join_path(root: &str, path: &str) -> String {
    if path.is_empty() {
        return root.to_string();
    }
    if root.is_empty() || is_absolute_url(path) || path.starts_with('/') {
        return path.to_string();
    }
    let trimmed = root.trim_end_matches('/');
    normalize_path(&format!("{}/{}", trimmed, path))
}

/// `util.relative()`: strips `root` from `path` when `path` sits inside it.
pub fn relative_path(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return path.to_string();
    }
    if let Some(rest) = path.strip_prefix(root) {
        if let Some(rest) = rest.strip_prefix('/') {
            return rest.to_string();
        }
        if rest.is_empty() {
            return String::new();
        }
    }
    path.to_string()
}

fn strip_prefix_path<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return None;
    }
    path.strip_prefix(root)?.strip_prefix('/')
}

/// Collapses `.` and `..` segments, keeping any leading `..`.
pub fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                if matches!(parts.last(), Some(&last) if last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_encodes_mappings() {
        let mappings = "AAAA,IAAM;IAAI,OAAO";
        let decoded = decode_mappings(mappings);
        assert_eq!(decoded.len(), 4);
        assert_eq!(decoded[0].generated_line, 1);
        assert_eq!(decoded[0].generated_column, 0);
        assert_eq!(decoded[0].original_line, Some(1));
        assert_eq!(decoded[1].generated_column, 4);
        assert_eq!(decoded[1].original_column, Some(6));
        assert_eq!(decoded[2].generated_line, 2);

        let mut generator = SourceMapGenerator::new(Some("out.css".into()));
        for mapping in &decoded {
            generator.add_mapping(GeneratedMapping {
                generated_line: mapping.generated_line,
                generated_column: mapping.generated_column,
                original_line: mapping.original_line,
                original_column: mapping.original_column,
                source: Some("a.css".into()),
                name: None,
            });
        }
        assert_eq!(generator.to_raw().mappings, mappings);
    }

    #[test]
    fn finds_original_positions() {
        let consumer = SourceMapConsumer::from_json(
            r#"{"version":3,"file":"b.css","sources":["a.css"],"names":[],"mappings":"AAAA,IAAM"}"#,
        )
        .unwrap();

        let found = consumer.original_position_for(1, 5);
        assert_eq!(found.source.as_deref(), Some("a.css"));
        assert_eq!(found.line, Some(1));
        assert_eq!(found.column, Some(6));

        // Nothing maps line 2.
        assert_eq!(consumer.original_position_for(2, 0).source, None);
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("a/./b/../c"), "a/c");
        assert_eq!(normalize_path("../a/b"), "../a/b");
        assert_eq!(join_path("dir", "file.css"), "dir/file.css");
        assert_eq!(join_path("dir", "/abs.css"), "/abs.css");
        assert_eq!(relative_path("dir", "dir/file.css"), "file.css");
        assert_eq!(relative_path("dir", "other/file.css"), "other/file.css");
    }
}
