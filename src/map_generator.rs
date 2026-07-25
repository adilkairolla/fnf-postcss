//! Building the output CSS together with its source map.
//!
//! Port of `lib/map-generator.js`. Handles inline and external maps, the
//! `sourceMappingURL` annotation, `sourcesContent`, and chaining onto the map of
//! a previous compilation step — which is what makes a Vite/webpack pipeline
//! point at the original `.scss` or `.vue` file instead of at intermediate CSS.

use std::collections::HashSet;
use std::sync::Arc;

use crate::input::{path_to_file_url, Input};
use crate::node::NodeId;
use crate::options::{Annotation, MapOptions, ProcessOptions};
use crate::previous_map::{dirname, PreviousMap};
use crate::source_map::{GeneratedMapping, SourceMapGenerator};
use crate::stringifier::{stringify, Part};
use crate::tree::Tree;
use crate::vlq::base64_encode;

const NO_SOURCE: &str = "<no source>";

/// Renders a tree, optionally producing a source map.
pub struct MapGenerator<'a> {
    opts: &'a ProcessOptions,
    map_opts: MapOptions,
    uses_file_urls: bool,
    css: String,
    map: SourceMapGenerator,
}

impl<'a> MapGenerator<'a> {
    /// Renders `tree`, returning the CSS and the map when one was requested.
    ///
    /// Removes any existing `sourceMappingURL` comment from the tree, as the JS
    /// version does.
    pub fn generate(
        tree: &mut Tree,
        opts: &'a ProcessOptions,
    ) -> (String, Option<SourceMapGenerator>) {
        let map_opts = opts.map.as_ref().map(|map| map.options()).unwrap_or_default();

        clear_annotation(tree, &map_opts);

        let previous = previous_maps(tree);
        let wants_map = match &opts.map {
            Some(setting) => setting.is_enabled(),
            None => !previous.is_empty(),
        };
        if !wants_map {
            return (tree.to_css(), None);
        }

        let mut generator = MapGenerator {
            opts,
            uses_file_urls: map_opts.from.is_none() && map_opts.absolute,
            map_opts,
            css: String::new(),
            map: SourceMapGenerator::new(None),
        };
        generator.map.file = Some(generator.output_file());
        generator.generate_string(tree);

        if generator.is_sources_content(&previous) {
            generator.set_sources_content(tree);
        }
        if !previous.is_empty() {
            generator.apply_previous_maps(&previous);
        }
        if generator.is_annotation(&previous) {
            generator.add_annotation(&previous);
        }

        if generator.is_inline(&previous) {
            (generator.css, None)
        } else {
            (generator.css, Some(generator.map))
        }
    }

    /// Stringifies the tree, recording a mapping for every node.
    fn generate_string(&mut self, tree: &Tree) {
        let mut line = 1usize;
        let mut column = 1usize;
        let mut css = String::new();
        let mut mappings: Vec<GeneratedMapping> = Vec::new();

        {
            let map_from = self.map_opts.from.clone();
            let uses_file_urls = self.uses_file_urls;
            let map_opts = self.map_opts.clone();
            let opts = self.opts;

            let source_path = |tree: &Tree, node: NodeId| -> String {
                let input = tree
                    .source(node)
                    .map(|source| Arc::clone(&source.input))
                    .expect("mapped node has a source");
                if let Some(from) = &map_from {
                    to_url(from)
                } else if uses_file_urls {
                    path_to_file_url(input.from())
                } else {
                    to_url(&path(input.from(), &map_opts, opts))
                }
            };

            let mut builder = |text: &str, node: Option<NodeId>, part: Option<Part>| {
                css.push_str(text);

                if let Some(node) = node {
                    if part != Some(Part::End) {
                        match tree.source(node).and_then(|source| source.start) {
                            Some(start) => mappings.push(GeneratedMapping {
                                generated_line: line,
                                generated_column: column - 1,
                                original_line: Some(start.line),
                                original_column: Some(start.column - 1),
                                source: Some(source_path(tree, node)),
                                name: None,
                            }),
                            None => mappings.push(GeneratedMapping {
                                generated_line: line,
                                generated_column: column - 1,
                                original_line: Some(1),
                                original_column: Some(0),
                                source: Some(NO_SOURCE.to_string()),
                                name: None,
                            }),
                        }
                    }
                }

                let newlines = text.matches('\n').count();
                if newlines > 0 {
                    line += newlines;
                    let after = text.rsplit('\n').next().unwrap_or("");
                    column = after.chars().count() + 1;
                } else {
                    column += text.chars().count();
                }

                if let Some(node) = node {
                    if part != Some(Part::Start) {
                        // A declaration or block-less at-rule that ends the
                        // block has no character of its own after the value, so
                        // it gets no end mapping unless a `;` follows.
                        let childless = tree.type_name(node) == "decl"
                            || (tree.type_name(node) == "atrule" && !tree.is_container(node));
                        let parent = tree.parent(node);
                        let is_last = parent.is_some_and(|parent| tree.last(parent) == Some(node));
                        let parent_semicolon = parent
                            .and_then(|parent| tree.raws(parent).semicolon)
                            .unwrap_or(false);

                        if !childless || !is_last || parent_semicolon {
                            match tree.source(node).and_then(|source| source.end) {
                                Some(end) => mappings.push(GeneratedMapping {
                                    generated_line: line,
                                    generated_column: column.saturating_sub(2),
                                    original_line: Some(end.line),
                                    original_column: Some(end.column - 1),
                                    source: Some(source_path(tree, node)),
                                    name: None,
                                }),
                                None => mappings.push(GeneratedMapping {
                                    generated_line: line,
                                    generated_column: column - 1,
                                    original_line: Some(1),
                                    original_column: Some(0),
                                    source: Some(NO_SOURCE.to_string()),
                                    name: None,
                                }),
                            }
                        }
                    }
                }
            };

            stringify(tree, tree.root(), &mut builder);
        }

        for mapping in mappings {
            self.map.add_mapping(mapping);
        }
        self.css = css;
    }

    fn output_file(&self) -> String {
        if let Some(to) = &self.opts.to {
            path(to, &self.map_opts, self.opts)
        } else if let Some(from) = &self.opts.from {
            path(from, &self.map_opts, self.opts)
        } else {
            "to.css".to_string()
        }
    }

    fn set_sources_content(&mut self, tree: &Tree) {
        let mut already: HashSet<String> = HashSet::new();
        let mut contents: Vec<(String, String)> = Vec::new();

        let mut record = |input: &Input| {
            let from = input.from().to_string();
            if from.is_empty() || already.contains(&from) {
                return;
            }
            already.insert(from.clone());
            let url = if self.uses_file_urls {
                path_to_file_url(&from)
            } else {
                to_url(&path(&from, &self.map_opts, self.opts))
            };
            contents.push((url, input.css().to_string()));
        };

        if let Some(source) = tree.source(tree.root()) {
            record(&source.input);
        }
        tree.walk_ref(tree.root(), |tree, node| {
            if let Some(source) = tree.source(node) {
                record(&source.input);
            }
        });

        for (url, content) in contents {
            self.map.set_source_content(&url, Some(content));
        }
    }

    /// Retargets our mappings through each input's own map.
    fn apply_previous_maps(&mut self, previous: &[Arc<Input>]) {
        for input in previous {
            let Some(prev) = &input.map else { continue };
            let Some(consumer) = prev.consumer() else {
                continue;
            };

            let prev_file = prev.file.clone().unwrap_or_default();
            let from = to_url(&path(&prev_file, &self.map_opts, self.opts));
            let root = prev
                .root
                .clone()
                .or_else(|| dirname(&prev_file))
                .unwrap_or_else(|| ".".to_string());
            let source_map_path = to_url(&path(&root, &self.map_opts, self.opts));

            if self.map_opts.sources_content == Some(false) {
                let mut stripped = consumer.clone();
                stripped.sources_content = None;
                self.map
                    .apply_source_map(&stripped, Some(&from), Some(&source_map_path));
            } else {
                self.map
                    .apply_source_map(consumer, Some(&from), Some(&source_map_path));
            }
        }
    }

    fn add_annotation(&mut self, previous: &[Arc<Input>]) {
        let content = if self.is_inline(previous) {
            format!(
                "data:application/json;base64,{}",
                base64_encode(self.map.to_json_string().as_bytes())
            )
        } else if let Some(Annotation::Path(path)) = &self.map_opts.annotation {
            path.clone()
        } else {
            format!("{}.map", self.output_file())
        };

        let eol = if self.css.contains("\r\n") { "\r\n" } else { "\n" };
        self.css
            .push_str(&format!("{}/*# sourceMappingURL={} */", eol, content));
    }

    fn is_annotation(&self, previous: &[Arc<Input>]) -> bool {
        if self.is_inline(previous) {
            return true;
        }
        match &self.map_opts.annotation {
            Some(Annotation::Disabled) => false,
            Some(_) => true,
            None => {
                if !previous.is_empty() {
                    previous
                        .iter()
                        .any(|input| input.map.as_ref().is_some_and(|map| map.annotation.is_some()))
                } else {
                    true
                }
            }
        }
    }

    fn is_inline(&self, previous: &[Arc<Input>]) -> bool {
        if let Some(inline) = self.map_opts.inline {
            return inline;
        }
        match &self.map_opts.annotation {
            Some(Annotation::Enabled) | None => {}
            // An explicit annotation path means the map is written separately.
            Some(_) => return false,
        }
        if !previous.is_empty() {
            return previous
                .iter()
                .any(|input| input.map.as_ref().is_some_and(|map| map.inline));
        }
        true
    }

    fn is_sources_content(&self, previous: &[Arc<Input>]) -> bool {
        if let Some(sources_content) = self.map_opts.sources_content {
            return sources_content;
        }
        if !previous.is_empty() {
            return previous
                .iter()
                .any(|input| input.map.as_ref().is_some_and(PreviousMap::with_content));
        }
        true
    }
}

/// Every input in the tree that carries a source map, in walk order.
fn previous_maps(tree: &Tree) -> Vec<Arc<Input>> {
    tree.inputs()
        .into_iter()
        .filter(|input| input.map.is_some())
        .collect()
}

/// Removes the `sourceMappingURL` comment the previous step left behind.
fn clear_annotation(tree: &mut Tree, map_opts: &MapOptions) {
    if matches!(map_opts.annotation, Some(Annotation::Disabled)) {
        return;
    }

    let root = tree.root();
    let children = tree.children(root).to_vec();
    for &node in children.iter().rev() {
        if tree.type_name(node) != "comment" {
            continue;
        }
        if tree
            .text(node)
            .is_some_and(|text| text.starts_with("# sourceMappingURL="))
        {
            tree.remove_child(root, node);
        }
    }
}

/// Makes a path relative to the output file, unless it is absolute by request,
/// a URL, or a placeholder such as `<no source>`.
fn path(file: &str, map_opts: &MapOptions, opts: &ProcessOptions) -> String {
    if map_opts.absolute {
        return file.to_string();
    }
    if file.starts_with('<') {
        return file.to_string();
    }
    if crate::input::is_url(file) {
        return file.to_string();
    }

    let mut from = match &opts.to {
        Some(to) => dirname(to).unwrap_or_else(|| ".".to_string()),
        None => ".".to_string(),
    };
    if let Some(Annotation::Path(annotation)) = &map_opts.annotation {
        let resolved = crate::previous_map::join(&from, annotation);
        from = dirname(&resolved).unwrap_or_else(|| ".".to_string());
    }

    crate::previous_map::relative(&from, file)
}

/// `encodeURI()` plus `#`/`?` escaping, so a path is safe inside a map.
fn to_url(path: &str) -> String {
    let mut url = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' | b';' | b'/'
            | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'[' | b']' => {
                url.push(byte as char)
            }
            other => url.push_str(&format!("%{:02X}", other)),
        }
    }
    url
}
