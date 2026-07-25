//! A Rust port of [PostCSS](https://postcss.org): parse CSS into an AST,
//! transform it with plugins, and stringify it back — with byte-exact
//! round-tripping and source map support.
//!
//! ```
//! use postcss::{parse, NewNode};
//!
//! let mut tree = parse("a { color: red }").unwrap();
//! tree.walk_decls(|tree, decl| {
//!     if tree.value(decl) == Some("red") {
//!         tree.set_value(decl, "green");
//!     }
//! });
//! assert_eq!(tree.to_css(), "a { color: green }");
//! ```
//!
//! # Differences from the JS implementation
//!
//! - Nodes are addressed by [`NodeId`] into a [`Tree`] arena instead of by
//!   reference, so mutation during a walk needs no garbage collector.
//! - `offset` is a UTF-8 byte offset and `column` counts characters; JS counts
//!   UTF-16 code units for both. They agree for ASCII.
//! - Plugins implement the [`Plugin`] trait. Everything runs synchronously,
//!   so there is no async plugin API.

#![warn(missing_docs)]
// `CssSyntaxError` carries the source text and full position information, like
// the error object in JS. That makes it larger than the lint's threshold, and
// boxing it would push a `Box` into every fallible signature for no benefit on
// the success path.
#![allow(clippy::result_large_err)]

pub mod error;
pub mod input;
pub mod list;
pub mod node;
pub mod options;
pub mod parser;
pub mod previous_map;
pub mod processor;
pub mod result;
pub mod source_map;
pub mod stringifier;
mod terminal_highlight;
pub mod tokenize;
pub mod tree;
mod vlq;

pub mod json;
pub mod map_generator;

use std::sync::Arc;

pub use error::CssSyntaxError;
pub use input::Input;
pub use node::{NewNode, NodeData, NodeId, NodeKind, Position, RawValue, Raws, Source};
pub use options::{Annotation, InputOptions, MapOptions, MapSetting, PrevMap, ProcessOptions};
pub use parser::Parser;
pub use previous_map::PreviousMap;
pub use processor::{Plugin, Processor};
pub use result::{Message, PluginContext, Result as ProcessResult, Warning};
pub use stringifier::{stringify, Build, Part, Stringifier};
pub use tokenize::{Token, TokenKind, Tokenizer, TokenizerOptions};
pub use tree::{Insertable, NodeErrorOptions, Tree, Visit};

/// Parses CSS into a [`Tree`].
///
/// ```
/// # use postcss::parse;
/// let tree = parse("a{}").unwrap();
/// assert_eq!(tree.children(tree.root()).len(), 1);
/// ```
pub fn parse(css: impl Into<String>) -> Result<Tree, CssSyntaxError> {
    parse_with_options(css, InputOptions::default())
}

/// Parses CSS with options, such as the file name used in error messages.
pub fn parse_with_options(
    css: impl Into<String>,
    opts: InputOptions,
) -> Result<Tree, CssSyntaxError> {
    let from = opts.from.clone();
    let input = Arc::new(Input::new(css, opts));
    let parser = Parser::new(&input);
    parser.parse().map_err(|mut error| {
        // A syntax error in a file with a preprocessor extension almost always
        // means the wrong parser was used.
        if let Some(from) = &from {
            let lowered = from.to_lowercase();
            let hint = if lowered.ends_with(".scss") {
                Some("\nYou tried to parse SCSS with the standard CSS parser; try again with the postcss-scss parser")
            } else if lowered.contains(".sass") {
                Some("\nYou tried to parse Sass with the standard CSS parser; try again with the postcss-sass parser")
            } else if lowered.ends_with(".less") {
                Some("\nYou tried to parse Less with the standard CSS parser; try again with the postcss-less parser")
            } else {
                None
            };
            if let Some(hint) = hint {
                error.message.push_str(hint);
            }
        }
        error
    })
}

/// Renders a tree to CSS.
pub fn stringify_tree(tree: &Tree) -> String {
    tree.to_css()
}
