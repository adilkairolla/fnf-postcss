//! Options accepted by [`crate::parse`], [`crate::Processor`] and
//! [`crate::Root::to_result`].

use std::path::PathBuf;

use crate::source_map::RawSourceMap;

/// Where a previous source map comes from.
///
/// Covers the `map.prev` forms of the JS API: `false`, a JSON string, a parsed
/// map object, or a path to read (what the function form resolves to).
#[derive(Clone, Debug)]
pub enum PrevMap {
    /// Ignore any previous map (`prev: false`).
    Disabled,
    /// Map JSON as text.
    Text(String),
    /// Already parsed map.
    Raw(Box<RawSourceMap>),
    /// Path to a map file, read eagerly and required to exist.
    File(PathBuf),
}

/// How the `/*# sourceMappingURL=... */` comment is written.
#[derive(Clone, Debug)]
pub enum Annotation {
    /// Do not add the comment.
    Disabled,
    /// Add the comment pointing at the default `<output>.map`.
    Enabled,
    /// Add the comment pointing at this path.
    Path(String),
}

/// Source map options (`opts.map` when it is an object).
#[derive(Clone, Debug, Default)]
pub struct MapOptions {
    /// Embed the map in the CSS as a `data:` URI.
    pub inline: Option<bool>,
    /// A map of an earlier compilation step to chain onto.
    pub prev: Option<PrevMap>,
    /// Embed the original CSS in `sourcesContent`.
    pub sources_content: Option<bool>,
    /// Whether and where to write the annotation comment.
    pub annotation: Option<Annotation>,
    /// Override the `sources` entry for every mapping.
    pub from: Option<String>,
    /// Use absolute `file:` URLs in `sources`.
    pub absolute: bool,
}

/// The tri-state `opts.map` of the JS API: `false`, `true`, or an object.
#[derive(Clone, Debug)]
pub enum MapSetting {
    /// `map: false` — never read or write maps.
    Disabled,
    /// `map: true` — write a map with default options.
    Enabled,
    /// `map: {...}`
    Options(MapOptions),
}

impl MapSetting {
    /// The options, or defaults for the `true`/`false` forms.
    pub fn options(&self) -> MapOptions {
        match self {
            MapSetting::Options(options) => options.clone(),
            _ => MapOptions::default(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        !matches!(self, MapSetting::Disabled)
    }
}

impl From<bool> for MapSetting {
    fn from(value: bool) -> Self {
        if value {
            MapSetting::Enabled
        } else {
            MapSetting::Disabled
        }
    }
}

impl From<MapOptions> for MapSetting {
    fn from(value: MapOptions) -> Self {
        MapSetting::Options(value)
    }
}

/// Options for building an [`crate::Input`].
#[derive(Clone, Debug, Default)]
pub struct InputOptions {
    /// Path of the CSS file being parsed. Used for error messages and to find
    /// a neighbouring source map.
    pub from: Option<String>,
    /// The enclosing document, for CSS embedded in another language.
    pub document: Option<String>,
    /// Source map handling.
    pub map: Option<MapSetting>,
    /// Allow reading map files that a strict resolver would reject, such as
    /// maps outside the CSS file's directory.
    pub unsafe_map: bool,
}

/// Options for a whole processing run.
#[derive(Clone, Debug, Default)]
pub struct ProcessOptions {
    /// Path of the input file.
    pub from: Option<String>,
    /// Path of the output file. Used to compute relative `sources` and the
    /// default annotation path.
    pub to: Option<String>,
    /// The enclosing document, for CSS embedded in another language.
    pub document: Option<String>,
    /// Source map handling.
    pub map: Option<MapSetting>,
    /// See [`InputOptions::unsafe_map`].
    pub unsafe_map: bool,
}

impl ProcessOptions {
    /// Options for a run that reads `from` and writes `to`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the input file path.
    pub fn from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// Sets the output file path.
    pub fn to(mut self, to: impl Into<String>) -> Self {
        self.to = Some(to.into());
        self
    }

    /// Sets source map handling.
    pub fn map(mut self, map: impl Into<MapSetting>) -> Self {
        self.map = Some(map.into());
        self
    }

    /// The [`InputOptions`] implied by these options.
    pub fn input_options(&self) -> InputOptions {
        InputOptions {
            from: self.from.clone(),
            document: self.document.clone(),
            map: self.map.clone(),
            unsafe_map: self.unsafe_map,
        }
    }
}
