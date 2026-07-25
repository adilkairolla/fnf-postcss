//! Node.js bindings for the Rust PostCSS core.
//!
//! Three entry points, matching what the JS layer in `node/lib` needs:
//!
//! - [`parse`] — CSS in, AST out. The AST crosses the boundary as real JS
//!   objects (napi's serde bridge), never as a JSON string, so the JS side does
//!   no parsing of its own.
//! - [`stringify`] — an AST back to CSS, for a tree the JS side has mutated.
//! - [`process`] — CSS in, CSS plus a source map out, for the map options a
//!   bundler passes through.
//!
//! Syntax errors come back as JS `Error`s whose message is a JSON payload with
//! everything `CssSyntaxError` needs, so the JS layer can rebuild a real one.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use postcss::{
    json, Annotation, InputOptions, MapOptions, MapSetting, PrevMap, ProcessOptions, Processor,
};
use serde_json::{Map, Value};

/// Options accepted by [`parse`] and [`process`].
#[napi(object)]
#[derive(Default)]
pub struct JsOptions {
    /// Path of the file the CSS came from.
    pub from: Option<String>,
    /// Path the output will be written to.
    pub to: Option<String>,
    /// `false` disables map handling entirely; otherwise see the map fields.
    pub map: Option<bool>,
    /// Inline the map into the CSS as a `data:` annotation.
    pub map_inline: Option<bool>,
    /// Include the sources' text in the map.
    pub map_sources_content: Option<bool>,
    /// `false` writes no annotation comment; a string sets its path.
    pub map_annotation: Option<Either<bool, String>>,
    /// A previous map to chain through, as JSON text.
    pub map_prev: Option<String>,
    /// Rewrite the map's `from` prefix.
    pub map_from: Option<String>,
    /// Keep absolute paths in the map's `sources`.
    pub map_absolute: Option<bool>,
}

/// What [`process`] returns.
#[napi(object)]
pub struct ProcessOutput {
    /// The transformed CSS.
    pub css: String,
    /// The map as JSON text, when one was generated and not inlined.
    pub map: Option<String>,
}

fn number_or_null(value: Option<usize>) -> Value {
    value.map_or(Value::Null, Value::from)
}

fn to_js_error(error: postcss::CssSyntaxError) -> Error {
    let mut payload = Map::new();
    payload.insert("__cssSyntaxError".into(), Value::Bool(true));
    payload.insert("reason".into(), Value::String(error.reason));
    payload.insert("message".into(), Value::String(error.message));
    payload.insert("file".into(), error.file.map_or(Value::Null, Value::String));
    payload.insert(
        "source".into(),
        error.source.map_or(Value::Null, Value::String),
    );
    payload.insert(
        "plugin".into(),
        error.plugin.map_or(Value::Null, Value::String),
    );
    payload.insert("line".into(), number_or_null(error.line));
    payload.insert("column".into(), number_or_null(error.column));
    payload.insert("endLine".into(), number_or_null(error.end_line));
    payload.insert("endColumn".into(), number_or_null(error.end_column));
    // `input` is the position inside the *original* file, after walking back
    // through any previous source map.
    payload.insert(
        "input".into(),
        match error.input {
            Some(input) => {
                let mut map = Map::new();
                map.insert("line".into(), Value::from(input.line));
                map.insert("column".into(), Value::from(input.column));
                map.insert("offset".into(), Value::from(input.offset));
                map.insert("endLine".into(), number_or_null(input.end_line));
                map.insert("endColumn".into(), number_or_null(input.end_column));
                map.insert("endOffset".into(), number_or_null(input.end_offset));
                map.insert("file".into(), input.file.map_or(Value::Null, Value::String));
                Value::Object(map)
            }
            None => Value::Null,
        },
    );
    Error::new(Status::GenericFailure, Value::Object(payload).to_string())
}

fn input_options(opts: &Option<JsOptions>) -> InputOptions {
    let mut input = InputOptions::default();
    if let Some(opts) = opts {
        input.from = opts.from.clone();
        input.map = map_setting(opts);
    }
    input
}

fn map_setting(opts: &JsOptions) -> Option<MapSetting> {
    if opts.map == Some(false) {
        return Some(MapSetting::Disabled);
    }
    let map = MapOptions {
        inline: opts.map_inline,
        sources_content: opts.map_sources_content,
        absolute: opts.map_absolute.unwrap_or(false),
        from: opts.map_from.clone(),
        prev: opts.map_prev.clone().map(PrevMap::Text),
        annotation: match &opts.map_annotation {
            Some(Either::A(true)) => Some(Annotation::Enabled),
            Some(Either::A(false)) => Some(Annotation::Disabled),
            Some(Either::B(path)) => Some(Annotation::Path(path.clone())),
            None => None,
        },
    };

    let untouched = map.inline.is_none()
        && map.sources_content.is_none()
        && map.annotation.is_none()
        && map.prev.is_none()
        && map.from.is_none()
        && !map.absolute;
    match (opts.map, untouched) {
        // `map: true` with no further detail means "defaults".
        (Some(true), true) => Some(MapSetting::Enabled),
        (_, true) => None,
        _ => Some(MapSetting::Options(map)),
    }
}

/// Parses CSS and returns the AST as JS objects.
#[napi]
pub fn parse(css: String, opts: Option<JsOptions>) -> Result<Value> {
    let tree = postcss::parse_with_options(css, input_options(&opts)).map_err(to_js_error)?;
    Ok(json::to_json(&tree))
}

/// Stringifies an AST — the shape [`parse`] returns, after the JS side has had
/// its way with it — back to CSS.
#[napi]
pub fn stringify(ast: Value) -> Result<String> {
    let tree = json::from_json(&ast).map_err(Error::from_reason)?;
    Ok(postcss::stringify_tree(&tree))
}

/// Stringifies an AST and generates its source map together.
///
/// Map generation has to walk the tree recording where every node landed, which
/// is the same walk stringifying does — so the two happen in one pass here,
/// rather than the JS side asking twice.
#[napi]
pub fn stringify_with_map(ast: Value, opts: Option<JsOptions>) -> Result<ProcessOutput> {
    let mut tree = json::from_json(&ast).map_err(Error::from_reason)?;
    let options = ProcessOptions {
        from: opts.as_ref().and_then(|opts| opts.from.clone()),
        to: opts.as_ref().and_then(|opts| opts.to.clone()),
        map: opts.as_ref().and_then(map_setting),
        ..Default::default()
    };
    let (css, map) = postcss::map_generator::MapGenerator::generate(&mut tree, &options);
    Ok(ProcessOutput {
        css,
        map: map.map(|map| map.to_json_string()),
    })
}

/// Tokenizes CSS, for the error-snippet highlighting on the JS side.
///
/// Each token is `[type, content, start, end]`, with the positions omitted when
/// the tokenizer does not record them — the shape `tokenize.js` produces.
#[napi]
pub fn tokenize(css: String) -> Vec<Vec<Value>> {
    let input = postcss::Input::from_css(css);
    let mut tokenizer = postcss::Tokenizer::new(&input, postcss::TokenizerOptions::default());
    let mut tokens = Vec::new();
    // Highlighting is best-effort: broken CSS is exactly when it runs, so stop
    // at the first token the tokenizer refuses rather than failing the error.
    while let Ok(Some(token)) = tokenizer.next_token(false) {
        let mut entry = vec![
            Value::String(token.kind.as_str().to_string()),
            Value::String(token.content.to_string()),
        ];
        if let Some(start) = token.start {
            entry.push(Value::from(start));
            if let Some(end) = token.end {
                entry.push(Value::from(end));
            }
        }
        tokens.push(entry);
    }
    tokens
}

/// Parses, stringifies and generates a source map in one call, for when no
/// plugin needs to see the tree.
#[napi]
pub fn process(css: String, opts: Option<JsOptions>) -> Result<ProcessOutput> {
    let options = ProcessOptions {
        from: opts.as_ref().and_then(|opts| opts.from.clone()),
        to: opts.as_ref().and_then(|opts| opts.to.clone()),
        map: opts.as_ref().and_then(map_setting),
        ..Default::default()
    };
    let result = Processor::new()
        .process(css, options)
        .map_err(to_js_error)?;
    let map = result.map_json();
    Ok(ProcessOutput {
        css: result.css,
        map,
    })
}

/// One entry of a source map, as `originalPositionFor` returns it.
#[napi(object)]
pub struct OriginalPosition {
    /// The original file.
    pub source: Option<String>,
    /// 1-based line in the original file.
    pub line: Option<u32>,
    /// 0-based column in the original file.
    pub column: Option<u32>,
    /// Identifier name, when the map records one.
    pub name: Option<String>,
}

/// Reads positions out of an existing source map.
///
/// Stands in for `source-map-js`'s `SourceMapConsumer` on the JS side, so the
/// package needs no source-map dependency.
#[napi]
pub struct MapConsumer {
    inner: postcss::source_map::SourceMapConsumer,
}

#[napi]
impl MapConsumer {
    /// Parses a map from its JSON text.
    #[napi(constructor)]
    pub fn new(text: String) -> Result<Self> {
        let inner =
            postcss::source_map::SourceMapConsumer::from_json(&text).map_err(Error::from_reason)?;
        Ok(MapConsumer { inner })
    }

    /// The generated file the map describes.
    #[napi(getter)]
    pub fn file(&self) -> Option<String> {
        self.inner.file.clone()
    }

    /// Prefix prepended to every source.
    #[napi(getter)]
    pub fn source_root(&self) -> Option<String> {
        self.inner.source_root.clone()
    }

    /// Sources as written in the map, before `sourceRoot` is applied.
    #[napi(getter)]
    pub fn sources(&self) -> Vec<String> {
        self.inner.raw_sources.clone()
    }

    /// The sources' text, when the map carries it.
    #[napi(getter)]
    pub fn sources_content(&self) -> Option<Vec<Option<String>>> {
        self.inner.sources_content.clone()
    }

    /// Position in the original file that generated `line`/`column`.
    ///
    /// `line` is 1-based and `column` 0-based, as in `source-map-js`.
    #[napi]
    pub fn original_position_for(&self, line: u32, column: u32) -> OriginalPosition {
        let found = self
            .inner
            .original_position_for(line as usize, column as usize);
        OriginalPosition {
            source: found.source,
            line: found.line.map(|line| line as u32),
            column: found.column.map(|column| column as u32),
            name: found.name,
        }
    }

    /// The text of one source, when the map carries it.
    #[napi]
    pub fn source_content_for(&self, source: String) -> Option<String> {
        self.inner
            .source_content_for(&source)
            .map(|text| text.to_string())
    }

    /// The map back as JSON text, for `SourceMapGenerator.fromSourceMap`.
    #[napi]
    pub fn to_json_string(&self) -> String {
        self.inner.to_generator().to_json_string()
    }
}

/// Splits a CSS value on top-level commas.
#[napi]
pub fn list_comma(value: String) -> Vec<String> {
    postcss::list::comma(&value)
}

/// Splits a CSS value on top-level whitespace.
#[napi]
pub fn list_space(value: String) -> Vec<String> {
    postcss::list::space(&value)
}

/// Version of the Rust core, so the JS layer can report it.
#[napi]
pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
