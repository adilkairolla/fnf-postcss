//! AST node data.
//!
//! Ports the data side of `lib/node.js` and its subclasses (`root.js`,
//! `rule.js`, `at-rule.js`, `declaration.js`, `comment.js`, `document.js`).
//! Behaviour that needs the whole tree — insertion, walking, cloning — lives on
//! [`crate::Tree`].

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::input::Input;

/// Handle to a node inside a [`crate::Tree`].
///
/// Ids stay valid for the lifetime of the tree, including after the node is
/// detached, so a plugin can hold one across mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    /// The arena index behind this id.
    pub fn index(self) -> usize {
        self.0
    }
}

/// The node types of the CSS AST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// `@media (min-width: 100px) { … }` or `@charset "utf-8";`
    AtRule {
        /// The name after the `@`.
        name: String,
        /// Everything between the name and the `{` or `;`.
        params: String,
    },
    /// `/* comment */`
    Comment {
        /// The text between the delimiters, trimmed.
        text: String,
    },
    /// `color: red !important`
    Decl {
        /// The property name.
        prop: String,
        /// The value, with comments removed.
        value: String,
        /// Whether `!important` was present.
        important: bool,
    },
    /// A container of roots, for CSS extracted from another language.
    Document,
    /// The tree root.
    Root,
    /// `a.b > c { … }`
    Rule {
        /// The selector list, with comments removed.
        selector: String,
    },
}

impl NodeKind {
    /// The `type` string PostCSS uses.
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::AtRule { .. } => "atrule",
            NodeKind::Comment { .. } => "comment",
            NodeKind::Decl { .. } => "decl",
            NodeKind::Document => "document",
            NodeKind::Root => "root",
            NodeKind::Rule { .. } => "rule",
        }
    }
}

/// A `{ raw, value }` pair: the source text of a value and its cleaned form.
///
/// The stringifier prints `raw` only while `value` still matches, so writing to
/// the node's value automatically discards the stale raw text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawValue {
    /// The text exactly as it appeared in the source.
    pub raw: String,
    /// The cleaned value this raw text corresponds to.
    pub value: String,
}

/// Whitespace, comments and other source text that carries no semantics but must
/// be preserved to round-trip the input.
///
/// Every field is `None` when absent, which the stringifier distinguishes from
/// an empty string: absent means "infer from the rest of the document", empty
/// means "there really is nothing here".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Raws {
    /// Whitespace before the node.
    pub before: Option<String>,
    /// Whitespace after the last child, before the closing brace.
    pub after: Option<String>,
    /// Text between a selector/prop and the `{`/`:`.
    pub between: Option<String>,
    /// Text between an at-rule's name and its params.
    pub after_name: Option<String>,
    /// Text between `/*` and a comment's text.
    pub left: Option<String>,
    /// Text between a comment's text and `*/`.
    pub right: Option<String>,
    /// The `!important` text as written, when it is not exactly `" !important"`.
    pub important: Option<String>,
    /// A `;` that followed a rule's closing brace.
    pub own_semicolon: Option<String>,
    /// Whether the last child is followed by a `;`.
    pub semicolon: Option<bool>,
    /// Indentation, set on roots by custom syntaxes.
    pub indent: Option<String>,
    /// Source text of a declaration's value, when it differs from `value`.
    pub value: Option<RawValue>,
    /// Source text of an at-rule's params, when it differs from `params`.
    pub params: Option<RawValue>,
    /// Source text of a rule's selector, when it differs from `selector`.
    pub selector: Option<RawValue>,
    /// Keys a custom syntax or plugin added.
    pub extra: BTreeMap<String, Value>,
}

impl Raws {
    /// Reads a string-valued raw by its JS key name.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match key {
            "before" => self.before.as_deref(),
            "after" => self.after.as_deref(),
            "between" => self.between.as_deref(),
            "afterName" => self.after_name.as_deref(),
            "left" => self.left.as_deref(),
            "right" => self.right.as_deref(),
            "important" => self.important.as_deref(),
            "ownSemicolon" => self.own_semicolon.as_deref(),
            "indent" => self.indent.as_deref(),
            other => self.extra.get(other).and_then(|value| value.as_str()),
        }
    }

    /// Writes a string-valued raw by its JS key name.
    pub fn set_str(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match key {
            "before" => self.before = Some(value),
            "after" => self.after = Some(value),
            "between" => self.between = Some(value),
            "afterName" => self.after_name = Some(value),
            "left" => self.left = Some(value),
            "right" => self.right = Some(value),
            "important" => self.important = Some(value),
            "ownSemicolon" => self.own_semicolon = Some(value),
            "indent" => self.indent = Some(value),
            other => {
                self.extra.insert(other.to_string(), Value::String(value));
            }
        }
    }

    /// Removes a raw by its JS key name.
    pub fn remove(&mut self, key: &str) {
        match key {
            "before" => self.before = None,
            "after" => self.after = None,
            "between" => self.between = None,
            "afterName" => self.after_name = None,
            "left" => self.left = None,
            "right" => self.right = None,
            "important" => self.important = None,
            "ownSemicolon" => self.own_semicolon = None,
            "indent" => self.indent = None,
            "semicolon" => self.semicolon = None,
            "value" => self.value = None,
            "params" => self.params = None,
            "selector" => self.selector = None,
            other => {
                self.extra.remove(other);
            }
        }
    }

    /// The `{ raw, value }` pair for a value-like property.
    pub fn raw_value(&self, key: &str) -> Option<&RawValue> {
        match key {
            "value" => self.value.as_ref(),
            "params" => self.params.as_ref(),
            "selector" => self.selector.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn set_raw_value(&mut self, key: &str, value: RawValue) {
        match key {
            "value" => self.value = Some(value),
            "params" => self.params = Some(value),
            "selector" => self.selector = Some(value),
            _ => {}
        }
    }
}

/// A position in a source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    /// 1-based line.
    pub line: usize,
    /// 1-based character column.
    pub column: usize,
    /// UTF-8 byte offset.
    pub offset: usize,
}

/// Where a node came from.
#[derive(Clone, Debug)]
pub struct Source {
    /// The file this node was parsed from.
    pub input: Arc<Input>,
    /// Position of the node's first character.
    pub start: Option<Position>,
    /// Position of the node's last character, inclusive; `offset` is exclusive.
    pub end: Option<Position>,
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        // Two sources match when they describe the same span of the same input.
        Arc::ptr_eq(&self.input, &other.input) && self.start == other.start && self.end == other.end
    }
}

/// One node in the arena.
#[derive(Clone, Debug)]
pub struct NodeData {
    /// The node's type and type-specific fields.
    pub kind: NodeKind,
    /// Source text with no semantic meaning.
    pub raws: Raws,
    /// Where the node was parsed from.
    pub source: Option<Source>,
    pub(crate) parent: Option<NodeId>,
    /// `Some` for containers, including empty ones. An at-rule without a block
    /// keeps `None`, which is what makes `@charset "utf-8";` print without
    /// braces.
    pub(crate) nodes: Option<Vec<NodeId>>,
    pub(crate) is_clean: bool,
    /// Live cursors for in-progress `each()`/`walk()` calls, so inserting and
    /// removing during a walk shifts the iteration the same way it does in JS.
    ///
    /// A cursor is signed: removing the child a cursor sits on moves it to -1,
    /// so the following `+= 1` lands back on index 0 and the new first child is
    /// still visited.
    pub(crate) indexes: Vec<(u32, isize)>,
    pub(crate) last_each: u32,
}

impl NodeData {
    pub(crate) fn new(kind: NodeKind) -> Self {
        let nodes = match kind {
            NodeKind::Root | NodeKind::Document | NodeKind::Rule { .. } => Some(Vec::new()),
            _ => None,
        };
        NodeData {
            kind,
            raws: Raws::default(),
            source: None,
            parent: None,
            nodes,
            is_clean: false,
            indexes: Vec::new(),
            last_each: 0,
        }
    }

    /// The `type` string PostCSS uses.
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }

    /// True for nodes that can hold children, i.e. everything but declarations,
    /// comments and block-less at-rules.
    pub fn is_container(&self) -> bool {
        self.nodes.is_some()
    }

    pub(crate) fn index_slot(&self, iterator: u32) -> Option<isize> {
        self.indexes
            .iter()
            .find(|(id, _)| *id == iterator)
            .map(|(_, index)| *index)
    }

    pub(crate) fn bump_index_slot(&mut self, iterator: u32, by: isize) {
        if let Some(entry) = self.indexes.iter_mut().find(|(id, _)| *id == iterator) {
            entry.1 += by;
        }
    }

    pub(crate) fn set_index_slot(&mut self, iterator: u32, index: isize) {
        match self.indexes.iter_mut().find(|(id, _)| *id == iterator) {
            Some(entry) => entry.1 = index,
            None => self.indexes.push((iterator, index)),
        }
    }

    pub(crate) fn drop_index_slot(&mut self, iterator: u32) {
        self.indexes.retain(|(id, _)| *id != iterator);
    }
}

/// A node to be created and inserted, as accepted by
/// [`crate::Tree::append`] and friends.
///
/// ```
/// # use postcss::{NewNode, parse};
/// let mut tree = parse("a {}").unwrap();
/// let rule = tree.first(tree.root()).unwrap();
/// tree.append(rule, NewNode::decl("color", "red")).unwrap();
/// // The parsed `a {}` had nothing between its braces, so there is no
/// // trailing newline to reuse before the `}`.
/// assert_eq!(tree.to_css(), "a {\n    color: red}");
/// ```
#[derive(Clone, Debug)]
pub struct NewNode {
    /// The node's type and type-specific fields.
    pub kind: NodeKind,
    /// Whitespace and other source text to use.
    pub raws: Raws,
    /// Source position to attribute the node to.
    pub source: Option<Source>,
    /// Children, for container nodes.
    pub nodes: Option<Vec<NewNode>>,
}

impl NewNode {
    /// `prop: value`
    pub fn decl(prop: impl Into<String>, value: impl Into<String>) -> Self {
        NewNode::of(NodeKind::Decl {
            prop: prop.into(),
            value: value.into(),
            important: false,
        })
    }

    /// `selector { }`
    pub fn rule(selector: impl Into<String>) -> Self {
        let mut node = NewNode::of(NodeKind::Rule {
            selector: selector.into(),
        });
        node.nodes = Some(Vec::new());
        node
    }

    /// `@name params` — without a block unless [`NewNode::child`] is used.
    pub fn at_rule(name: impl Into<String>, params: impl Into<String>) -> Self {
        NewNode::of(NodeKind::AtRule {
            name: name.into(),
            params: params.into(),
        })
    }

    /// `/* text */`
    pub fn comment(text: impl Into<String>) -> Self {
        NewNode::of(NodeKind::Comment { text: text.into() })
    }

    /// An empty root.
    pub fn root() -> Self {
        NewNode::of(NodeKind::Root)
    }

    fn of(kind: NodeKind) -> Self {
        let nodes = match kind {
            NodeKind::Root | NodeKind::Document | NodeKind::Rule { .. } => Some(Vec::new()),
            _ => None,
        };
        NewNode {
            kind,
            raws: Raws::default(),
            source: None,
            nodes,
        }
    }

    /// Marks a declaration `!important`.
    pub fn important(mut self, important: bool) -> Self {
        if let NodeKind::Decl { important: flag, .. } = &mut self.kind {
            *flag = important;
        }
        self
    }

    /// Adds a child, giving an at-rule a block if it had none.
    pub fn child(mut self, child: NewNode) -> Self {
        self.nodes.get_or_insert_with(Vec::new).push(child);
        self
    }

    /// Sets `raws.before`.
    pub fn before(mut self, before: impl Into<String>) -> Self {
        self.raws.before = Some(before.into());
        self
    }

    /// Sets `raws.between`.
    pub fn between(mut self, between: impl Into<String>) -> Self {
        self.raws.between = Some(between.into());
        self
    }

    /// Replaces the whole raws set.
    pub fn raws(mut self, raws: Raws) -> Self {
        self.raws = raws;
        self
    }
}
