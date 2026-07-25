//! The AST itself: an arena of nodes plus every operation that needs to see
//! more than one node.
//!
//! Ports `lib/container.js` and the tree-aware half of `lib/node.js`.
//!
//! Nodes are addressed by [`NodeId`] rather than by reference, which is what
//! lets a plugin hold a handle to a node while mutating its neighbours — the
//! same freedom the JS API gets from garbage collection.

use std::collections::HashSet;
use std::sync::Arc;

use crate::error::CssSyntaxError;
use crate::input::{Input, Loc};
use crate::node::{NewNode, NodeData, NodeId, NodeKind, Position, RawValue, Raws, Source};
use crate::stringifier::{stringify_to_string, Stringifier};

/// Whether a walk should continue or stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visit {
    /// Keep walking.
    Continue,
    /// Stop the walk.
    Break,
}

/// Lets walk callbacks return `()`, `bool` or [`Visit`].
///
/// `false` breaks the walk, matching `return false` in the JS API.
pub trait IntoVisit {
    /// Converts the callback's return value into a walk decision.
    fn into_visit(self) -> Visit;
}

impl IntoVisit for () {
    fn into_visit(self) -> Visit {
        Visit::Continue
    }
}

impl IntoVisit for bool {
    fn into_visit(self) -> Visit {
        if self {
            Visit::Continue
        } else {
            Visit::Break
        }
    }
}

impl IntoVisit for Visit {
    fn into_visit(self) -> Visit {
        self
    }
}

/// Something that can be inserted into a tree.
///
/// Mirrors the values `container.append()` accepts in JS: a CSS string, a node
/// description, an existing node, a whole tree, or a list of those.
#[derive(Clone, Debug)]
pub enum Insertable {
    /// CSS to parse. The parsed nodes lose their source positions, as in JS.
    Css(String),
    /// A node to create.
    New(Box<NewNode>),
    /// A node already in this tree; it is moved.
    Node(NodeId),
    /// Another tree, whose children are adopted.
    Tree(Box<Tree>),
    /// Several of the above.
    Many(Vec<Insertable>),
}

impl From<&str> for Insertable {
    fn from(value: &str) -> Self {
        Insertable::Css(value.to_string())
    }
}

impl From<String> for Insertable {
    fn from(value: String) -> Self {
        Insertable::Css(value)
    }
}

impl From<NewNode> for Insertable {
    fn from(value: NewNode) -> Self {
        Insertable::New(Box::new(value))
    }
}

impl From<NodeId> for Insertable {
    fn from(value: NodeId) -> Self {
        Insertable::Node(value)
    }
}

impl From<Tree> for Insertable {
    fn from(value: Tree) -> Self {
        Insertable::Tree(Box::new(value))
    }
}

impl<T: Into<Insertable>> From<Vec<T>> for Insertable {
    fn from(value: Vec<T>) -> Self {
        Insertable::Many(value.into_iter().map(Into::into).collect())
    }
}

/// Options for [`Tree::node_error`] and [`crate::Result::warn`].
#[derive(Clone, Debug, Default)]
pub struct NodeErrorOptions {
    /// Name of the plugin reporting the problem.
    pub plugin: Option<String>,
    /// Highlight the first occurrence of this word inside the node.
    pub word: Option<String>,
    /// Highlight the character at this index inside the node.
    pub index: Option<usize>,
    /// End of the highlighted range.
    pub end_index: Option<usize>,
    /// Explicit start position, as `(line, column)`.
    pub start: Option<(usize, usize)>,
    /// Explicit end position, as `(line, column)`.
    pub end: Option<(usize, usize)>,
}

/// A CSS syntax tree.
///
/// ```
/// # use postcss::parse;
/// let mut tree = parse("a { color: red }").unwrap();
/// tree.walk_decls(|tree, decl| {
///     tree.set_value(decl, "blue");
/// });
/// assert_eq!(tree.to_css(), "a { color: blue }");
/// ```
#[derive(Clone, Debug)]
pub struct Tree {
    arena: Vec<NodeData>,
    root: NodeId,
}

impl Tree {
    /// A tree holding an empty root.
    pub fn new() -> Self {
        let mut arena = Vec::with_capacity(16);
        arena.push(NodeData::new(NodeKind::Root));
        Tree {
            arena,
            root: NodeId(0),
        }
    }

    /// A tree holding an empty document, for CSS extracted from another
    /// language.
    pub fn new_document() -> Self {
        let mut arena = Vec::with_capacity(16);
        arena.push(NodeData::new(NodeKind::Document));
        Tree {
            arena,
            root: NodeId(0),
        }
    }

    /// The root (or document) node.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Number of nodes ever allocated, including detached ones.
    pub fn len(&self) -> usize {
        self.arena.len()
    }

    /// True when the root has no children.
    pub fn is_empty(&self) -> bool {
        self.children(self.root).is_empty()
    }

    /// Read access to a node.
    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.arena[id.0]
    }

    /// Write access to a node.
    ///
    /// Changing a value through this does not mark the tree dirty; use the
    /// typed setters, or call [`Tree::mark_dirty`] afterwards, when a plugin
    /// visitor must see the change.
    pub fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.arena[id.0]
    }

    /// The node's `raws`.
    pub fn raws(&self, id: NodeId) -> &Raws {
        &self.arena[id.0].raws
    }

    /// Mutable access to the node's `raws`.
    pub fn raws_mut(&mut self, id: NodeId) -> &mut Raws {
        &mut self.arena[id.0].raws
    }

    /// The node's source position, if it was parsed.
    pub fn source(&self, id: NodeId) -> Option<&Source> {
        self.arena[id.0].source.as_ref()
    }

    /// The node's type name: `root`, `rule`, `atrule`, `decl`, `comment` or
    /// `document`.
    pub fn type_name(&self, id: NodeId) -> &'static str {
        self.arena[id.0].type_name()
    }

    /// The node's kind, with its type-specific fields.
    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.arena[id.0].kind
    }

    // --- Structure -------------------------------------------------------

    /// The node's parent, or `None` for a root or a detached node.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena[id.0].parent
    }

    /// The node's children, empty for a non-container.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        match &self.arena[id.0].nodes {
            Some(nodes) => nodes,
            None => &[],
        }
    }

    /// `Some` for containers, including empty ones; `None` for declarations,
    /// comments and at-rules without a block.
    pub fn nodes(&self, id: NodeId) -> Option<&[NodeId]> {
        self.arena[id.0].nodes.as_deref()
    }

    /// True when the node can hold children.
    pub fn is_container(&self, id: NodeId) -> bool {
        self.arena[id.0].nodes.is_some()
    }

    /// Gives an at-rule a block, so children can be appended to it.
    pub fn make_container(&mut self, id: NodeId) {
        if self.arena[id.0].nodes.is_none() {
            self.arena[id.0].nodes = Some(Vec::new());
        }
    }

    /// First child.
    pub fn first(&self, id: NodeId) -> Option<NodeId> {
        self.children(id).first().copied()
    }

    /// Last child.
    pub fn last(&self, id: NodeId) -> Option<NodeId> {
        self.children(id).last().copied()
    }

    /// Position of a child inside its parent.
    pub fn index(&self, parent: NodeId, child: NodeId) -> Option<usize> {
        self.children(parent).iter().position(|&node| node == child)
    }

    /// Next sibling.
    pub fn next(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        let index = self.index(parent, id)?;
        self.children(parent).get(index + 1).copied()
    }

    /// Previous sibling.
    pub fn prev(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        let index = self.index(parent, id)?;
        if index == 0 {
            return None;
        }
        self.children(parent).get(index - 1).copied()
    }

    /// The node's root, stopping at a document rather than crossing it.
    pub fn root_of(&self, id: NodeId) -> NodeId {
        let mut result = id;
        while let Some(parent) = self.parent(result) {
            if self.arena[parent.0].kind == NodeKind::Document {
                break;
            }
            result = parent;
        }
        result
    }

    // --- Typed field access ---------------------------------------------

    /// A rule's selector.
    pub fn selector(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::Rule { selector } => Some(selector),
            _ => None,
        }
    }

    /// Sets a rule's selector.
    pub fn set_selector(&mut self, id: NodeId, value: impl Into<String>) {
        if let NodeKind::Rule { selector } = &mut self.arena[id.0].kind {
            *selector = value.into();
            self.mark_dirty(id);
        }
    }

    /// A rule's selectors, split on top-level commas.
    pub fn selectors(&self, id: NodeId) -> Vec<String> {
        match self.selector(id) {
            Some(selector) => crate::list::comma(selector),
            None => Vec::new(),
        }
    }

    /// Joins selectors with the separator already used by this rule.
    pub fn set_selectors(&mut self, id: NodeId, values: &[impl AsRef<str>]) {
        let separator = match self.selector(id) {
            Some(selector) => match find_comma_separator(selector) {
                Some(separator) => separator,
                None => format!(",{}", self.raw(id, Some("between"), Some("beforeOpen"))),
            },
            None => ", ".to_string(),
        };
        let joined = values
            .iter()
            .map(|value| value.as_ref())
            .collect::<Vec<_>>()
            .join(&separator);
        self.set_selector(id, joined);
    }

    /// A declaration's property.
    pub fn prop(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::Decl { prop, .. } => Some(prop),
            _ => None,
        }
    }

    /// Sets a declaration's property.
    pub fn set_prop(&mut self, id: NodeId, value: impl Into<String>) {
        if let NodeKind::Decl { prop, .. } = &mut self.arena[id.0].kind {
            *prop = value.into();
            self.mark_dirty(id);
        }
    }

    /// A declaration's value.
    pub fn value(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::Decl { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Sets a declaration's value.
    ///
    /// The stored raw value is kept but stops being printed, since it no longer
    /// matches — the same rule the JS stringifier uses.
    pub fn set_value(&mut self, id: NodeId, new_value: impl Into<String>) {
        if let NodeKind::Decl { value, .. } = &mut self.arena[id.0].kind {
            *value = new_value.into();
            self.mark_dirty(id);
        }
    }

    /// Whether a declaration is `!important`.
    pub fn important(&self, id: NodeId) -> bool {
        match &self.arena[id.0].kind {
            NodeKind::Decl { important, .. } => *important,
            _ => false,
        }
    }

    /// Sets the `!important` flag.
    pub fn set_important(&mut self, id: NodeId, value: bool) {
        if let NodeKind::Decl { important, .. } = &mut self.arena[id.0].kind {
            *important = value;
            self.mark_dirty(id);
        }
    }

    /// True for custom properties and Sass-style `$variables`.
    pub fn is_variable(&self, id: NodeId) -> bool {
        match self.prop(id) {
            Some(prop) => prop.starts_with("--") || prop.starts_with('$'),
            None => false,
        }
    }

    /// An at-rule's name, without the `@`.
    pub fn name(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::AtRule { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Sets an at-rule's name.
    pub fn set_name(&mut self, id: NodeId, value: impl Into<String>) {
        if let NodeKind::AtRule { name, .. } = &mut self.arena[id.0].kind {
            *name = value.into();
            self.mark_dirty(id);
        }
    }

    /// An at-rule's params.
    pub fn params(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::AtRule { params, .. } => Some(params),
            _ => None,
        }
    }

    /// Sets an at-rule's params.
    pub fn set_params(&mut self, id: NodeId, value: impl Into<String>) {
        if let NodeKind::AtRule { params, .. } = &mut self.arena[id.0].kind {
            *params = value.into();
            self.mark_dirty(id);
        }
    }

    /// A comment's text, without the delimiters.
    pub fn text(&self, id: NodeId) -> Option<&str> {
        match &self.arena[id.0].kind {
            NodeKind::Comment { text } => Some(text),
            _ => None,
        }
    }

    /// Sets a comment's text.
    pub fn set_text(&mut self, id: NodeId, value: impl Into<String>) {
        if let NodeKind::Comment { text } = &mut self.arena[id.0].kind {
            *text = value.into();
            self.mark_dirty(id);
        }
    }

    // --- Dirty tracking --------------------------------------------------

    /// Marks a node and its ancestors as needing another visitor pass.
    pub fn mark_dirty(&mut self, id: NodeId) {
        if !self.arena[id.0].is_clean {
            return;
        }
        self.arena[id.0].is_clean = false;
        let mut next = self.arena[id.0].parent;
        while let Some(parent) = next {
            self.arena[parent.0].is_clean = false;
            next = self.arena[parent.0].parent;
        }
    }

    pub(crate) fn mark_clean(&mut self, id: NodeId) {
        self.arena[id.0].is_clean = true;
    }

    pub(crate) fn is_clean(&self, id: NodeId) -> bool {
        self.arena[id.0].is_clean
    }

    fn mark_tree_dirty(&mut self, id: NodeId) {
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            self.arena[next.0].is_clean = false;
            if let Some(children) = &self.arena[next.0].nodes {
                stack.extend(children.iter().copied());
            }
        }
    }

    // --- Allocation ------------------------------------------------------

    fn alloc(&mut self, data: NodeData) -> NodeId {
        self.arena.push(data);
        NodeId(self.arena.len() - 1)
    }

    /// Creates a detached node in this tree.
    pub fn create(&mut self, node: NewNode) -> NodeId {
        let mut data = NodeData::new(node.kind);
        data.raws = node.raws;
        data.source = node.source;
        if node.nodes.is_some() && data.nodes.is_none() {
            data.nodes = Some(Vec::new());
        }
        let id = self.alloc(data);

        if let Some(children) = node.nodes {
            for child in children {
                let child_id = self.create(child);
                self.push_child(id, child_id);
            }
        }
        id
    }

    /// Copies a node (and its subtree) from another tree into this one.
    fn adopt(&mut self, other: &Tree, id: NodeId, clear_source: bool) -> NodeId {
        let source = other.arena[id.0].clone();
        let mut data = NodeData::new(source.kind.clone());
        data.raws = source.raws.clone();
        data.source = if clear_source {
            None
        } else {
            source.source.clone()
        };
        data.nodes = source.nodes.as_ref().map(|_| Vec::new());
        let new_id = self.alloc(data);

        if let Some(children) = &source.nodes {
            for &child in children {
                let child_id = self.adopt(other, child, clear_source);
                self.push_child(new_id, child_id);
            }
        }
        new_id
    }

    /// Appends a child with no normalization: no `before` is inferred and no
    /// existing parent is cleaned up.
    ///
    /// Port of `container.push()`, which the docs describe as being for
    /// parsers. Prefer [`Tree::append`].
    pub fn push_child_public(&mut self, parent: NodeId, child: NodeId) {
        self.push_child(parent, child)
    }

    /// Appends a child without any normalization. Used by the parser.
    pub(crate) fn push_child(&mut self, parent: NodeId, child: NodeId) {
        self.arena[child.0].parent = Some(parent);
        self.arena[parent.0]
            .nodes
            .get_or_insert_with(Vec::new)
            .push(child);
    }

    // --- Cloning ---------------------------------------------------------

    /// Deep-copies a node, returning a detached copy.
    ///
    /// The copy shares the original's [`Source`], like `node.clone()` in JS.
    pub fn clone_node(&mut self, id: NodeId) -> NodeId {
        self.clone_subtree(id, None)
    }

    fn clone_subtree(&mut self, id: NodeId, parent: Option<NodeId>) -> NodeId {
        let source = self.arena[id.0].clone();
        let mut data = NodeData::new(source.kind);
        data.raws = source.raws;
        data.source = source.source;
        data.parent = parent;
        data.nodes = source.nodes.as_ref().map(|_| Vec::new());
        let new_id = self.alloc(data);

        if let Some(children) = source.nodes {
            for child in children {
                let child_id = self.clone_subtree(child, Some(new_id));
                self.arena[new_id.0]
                    .nodes
                    .get_or_insert_with(Vec::new)
                    .push(child_id);
            }
        }
        new_id
    }

    /// Clones a node and inserts the copy right after it.
    pub fn clone_after(&mut self, id: NodeId) -> Result<NodeId, CssSyntaxError> {
        let cloned = self.clone_node(id);
        self.insert_after(id, cloned)?;
        Ok(cloned)
    }

    /// Clones a node and inserts the copy right before it.
    pub fn clone_before(&mut self, id: NodeId) -> Result<NodeId, CssSyntaxError> {
        let cloned = self.clone_node(id);
        self.insert_before(id, cloned)?;
        Ok(cloned)
    }

    // --- Insertion -------------------------------------------------------

    /// Appends children to a container.
    pub fn append(
        &mut self,
        parent: NodeId,
        children: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        self.make_container(parent);
        let items = split_top_level(children.into());
        for child in items {
            let last = self.last(parent);
            let nodes = self.normalize(parent, child, last, None)?;
            for node in nodes {
                self.arena[parent.0]
                    .nodes
                    .get_or_insert_with(Vec::new)
                    .push(node);
            }
        }
        self.mark_dirty(parent);
        Ok(self)
    }

    /// Prepends children to a container.
    pub fn prepend(
        &mut self,
        parent: NodeId,
        children: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        self.make_container(parent);
        let items: Vec<Insertable> = split_top_level(children.into()).into_iter().rev().collect();
        for child in items {
            let first = self.first(parent);
            let mut nodes = self.normalize(parent, child, first, Some(InsertKind::Prepend))?;
            nodes.reverse();
            let count = nodes.len();
            for node in nodes {
                self.arena[parent.0]
                    .nodes
                    .get_or_insert_with(Vec::new)
                    .insert(0, node);
            }
            let slots: Vec<u32> = self.arena[parent.0]
                .indexes
                .iter()
                .map(|(id, _)| *id)
                .collect();
            for slot in slots {
                self.arena[parent.0].bump_index_slot(slot, count as isize);
            }
        }
        self.mark_dirty(parent);
        Ok(self)
    }

    /// Inserts nodes after `exist`.
    pub fn insert_after(
        &mut self,
        exist: NodeId,
        add: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        let Some(parent) = self.parent(exist) else {
            return Ok(self);
        };
        self.insert(parent, Target::Node(exist), Side::After, add.into())
    }

    /// Inserts nodes after the child at `index`.
    pub fn insert_after_index(
        &mut self,
        parent: NodeId,
        index: usize,
        add: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        self.insert(parent, Target::Index(index), Side::After, add.into())
    }

    /// Inserts nodes before `exist`.
    pub fn insert_before(
        &mut self,
        exist: NodeId,
        add: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        let Some(parent) = self.parent(exist) else {
            return Ok(self);
        };
        self.insert(parent, Target::Node(exist), Side::Before, add.into())
    }

    /// Inserts nodes before the child at `index`.
    pub fn insert_before_index(
        &mut self,
        parent: NodeId,
        index: usize,
        add: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        self.insert(parent, Target::Index(index), Side::Before, add.into())
    }

    /// Shared body of the four insert methods.
    ///
    /// A node target is re-resolved after normalization, since normalizing can
    /// move nodes out of this same parent and shift the position; a numeric
    /// target is used as given, matching `index()` in JS.
    fn insert(
        &mut self,
        parent: NodeId,
        target: Target,
        side: Side,
        add: Insertable,
    ) -> Result<&mut Self, CssSyntaxError> {
        let exist_index = target.resolve(self, parent);
        let sample = self.children(parent).get(exist_index).copied();
        let kind = if side == Side::Before && exist_index == 0 {
            Some(InsertKind::Prepend)
        } else {
            None
        };

        let mut nodes = self.normalize(parent, add, sample, kind)?;
        nodes.reverse();

        let exist_index = target.resolve(self, parent);
        let at = match side {
            Side::Before => exist_index,
            Side::After => exist_index + 1,
        };
        let count = nodes.len();
        for node in nodes {
            let children = self.arena[parent.0].nodes.get_or_insert_with(Vec::new);
            let at = at.min(children.len());
            children.insert(at, node);
        }

        // Cursors at or after the insertion point move along with their node.
        let slots: Vec<(u32, isize)> = self.arena[parent.0].indexes.clone();
        let exist_index = exist_index as isize;
        for (slot, index) in slots {
            let shifts = match side {
                Side::Before => exist_index <= index,
                Side::After => exist_index < index,
            };
            if shifts {
                self.arena[parent.0].bump_index_slot(slot, count as isize);
            }
        }

        self.mark_dirty(parent);
        Ok(self)
    }

    /// Replaces a node with others, removing it when it is not among them.
    pub fn replace_with(
        &mut self,
        id: NodeId,
        replacements: impl Into<Insertable>,
    ) -> Result<&mut Self, CssSyntaxError> {
        if self.parent(id).is_none() {
            return Ok(self);
        }

        let items = split_top_level(replacements.into());
        let mut bookmark = id;
        let mut found_self = false;

        for item in items {
            if matches!(item, Insertable::Node(node) if node == id) {
                found_self = true;
                continue;
            }
            if found_self {
                self.insert_after(bookmark, item.clone())?;
                if let Insertable::Node(node) = item {
                    bookmark = node;
                } else if let Some(next) = self.next(bookmark) {
                    bookmark = next;
                }
            } else {
                self.insert_before(bookmark, item)?;
            }
        }

        if !found_self {
            self.remove(id);
        }
        Ok(self)
    }

    /// Turns an [`Insertable`] into nodes owned by this tree and parented to
    /// `parent`.
    ///
    /// Port of `Container#normalize()` plus the `Root#normalize()` override.
    fn normalize(
        &mut self,
        parent: NodeId,
        insert: Insertable,
        sample: Option<NodeId>,
        kind: Option<InsertKind>,
    ) -> Result<Vec<NodeId>, CssSyntaxError> {
        let is_root_parent = self.arena[parent.0].kind == NodeKind::Root;

        let parent_is_document = self.arena[parent.0].kind == NodeKind::Document;
        let (materialized, from_array) = self.materialize(insert, parent_is_document)?;

        // `Root#normalize()` leaves an explicit `before` on an incoming node
        // alone. Nodes that came from parsed CSS are not protected: their
        // `before` belongs to the document they were parsed from.
        let mut keep_before: HashSet<NodeId> = HashSet::new();
        let mut nodes: Vec<NodeId> = Vec::with_capacity(materialized.len());
        for (id, protects_before) in materialized {
            if is_root_parent
                && protects_before
                && self.arena[id.0].parent.is_none()
                && self.arena[id.0].raws.before.is_some()
            {
                keep_before.insert(id);
            }
            nodes.push(id);
        }

        if from_array {
            // Arrays detach their members first, skipping the root's
            // before-shifting.
            for &node in &nodes {
                if let Some(old_parent) = self.arena[node.0].parent {
                    self.remove_child_inner(old_parent, node, true);
                }
            }
        }

        for &node in &nodes {
            if let Some(old_parent) = self.arena[node.0].parent {
                self.remove_child_inner(old_parent, node, false);
            }
            if self.arena[node.0].is_clean {
                self.mark_tree_dirty(node);
            }
            if !is_root_parent && self.arena[node.0].raws.before.is_none() {
                if let Some(sample) = sample {
                    if let Some(before) = self.arena[sample.0].raws.before.clone() {
                        self.arena[node.0].raws.before = Some(strip_non_space(&before));
                    }
                }
            }
            self.arena[node.0].parent = Some(parent);
        }

        if is_root_parent {
            if let Some(sample) = sample {
                match kind {
                    Some(InsertKind::Prepend) => {
                        let children = self.children(parent).to_vec();
                        if children.len() > 1 {
                            let before = self.arena[children[1].0].raws.before.clone();
                            self.arena[sample.0].raws.before = before;
                        } else {
                            self.arena[sample.0].raws.before = None;
                        }
                    }
                    None => {
                        if self.first(parent) != Some(sample) {
                            let before = self.arena[sample.0].raws.before.clone();
                            for &node in &nodes {
                                if !keep_before.contains(&node) {
                                    self.arena[node.0].raws.before = before.clone();
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(nodes)
    }

    /// Materializes an [`Insertable`] into node ids.
    ///
    /// Each id is paired with whether an explicit `before` on it should survive
    /// a root's normalization, and the second return value reports whether the
    /// insertable was a list (which detaches its members up front).
    fn materialize(
        &mut self,
        insert: Insertable,
        parent_is_document: bool,
    ) -> Result<(Vec<(NodeId, bool)>, bool), CssSyntaxError> {
        match insert {
            Insertable::Css(css) => {
                let parsed = crate::parse(&css)?;
                let children = parsed.children(parsed.root()).to_vec();
                let nodes = children
                    .into_iter()
                    .map(|child| (self.adopt(&parsed, child, true), false))
                    .collect();
                Ok((nodes, true))
            }
            Insertable::New(node) => {
                let protects_before = node.raws.before.is_some();
                Ok((vec![(self.create(*node), protects_before)], false))
            }
            Insertable::Node(id) => {
                let protects_before = self.arena[id.0].raws.before.is_some();
                Ok((vec![(id, protects_before)], false))
            }
            Insertable::Tree(other) => {
                let other_root = other.root();
                // A root appended to anything but a document contributes its
                // children; a document adopts the root itself.
                let nodes =
                    if other.arena[other_root.0].kind == NodeKind::Root && !parent_is_document {
                        other
                            .children(other_root)
                            .to_vec()
                            .into_iter()
                            .map(|child| (self.adopt(&other, child, false), false))
                            .collect()
                    } else {
                        vec![(self.adopt(&other, other_root, false), false)]
                    };
                Ok((nodes, true))
            }
            Insertable::Many(items) => {
                let mut nodes = Vec::new();
                for item in items {
                    let (mut ids, _) = self.materialize(item, parent_is_document)?;
                    nodes.append(&mut ids);
                }
                Ok((nodes, true))
            }
        }
    }

    // --- Removal ---------------------------------------------------------

    /// Detaches a node from its parent.
    pub fn remove(&mut self, id: NodeId) -> &mut Self {
        if let Some(parent) = self.parent(id) {
            self.remove_child_inner(parent, id, false);
        }
        self.arena[id.0].parent = None;
        self
    }

    /// Removes a child of `parent`.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> &mut Self {
        self.remove_child_inner(parent, child, false);
        self
    }

    fn remove_child_inner(&mut self, parent: NodeId, child: NodeId, ignore: bool) {
        let Some(index) = self.index(parent, child) else {
            return;
        };

        // `Root#removeChild()`: dropping the first node hands its `before` to
        // the next one, so the output does not gain leading whitespace.
        if !ignore
            && self.arena[parent.0].kind == NodeKind::Root
            && index == 0
            && self.children(parent).len() > 1
        {
            let children = self.children(parent).to_vec();
            let before = self.arena[children[0].0].raws.before.clone();
            self.arena[children[1].0].raws.before = before;
        }

        self.arena[child.0].parent = None;
        if let Some(nodes) = &mut self.arena[parent.0].nodes {
            nodes.remove(index);
        }

        let slots: Vec<(u32, isize)> = self.arena[parent.0].indexes.clone();
        for (slot, slot_index) in slots {
            if slot_index >= index as isize {
                self.arena[parent.0].bump_index_slot(slot, -1);
            }
        }

        self.mark_dirty(parent);
    }

    /// Removes the child at `index`.
    pub fn remove_child_at(&mut self, parent: NodeId, index: usize) -> &mut Self {
        if let Some(&child) = self.children(parent).get(index) {
            self.remove_child_inner(parent, child, false);
        }
        self
    }

    /// Reorders a container's children.
    ///
    /// Stands in for `container.nodes.sort()` in JS, which is possible there
    /// because `nodes` is a plain array.
    pub fn sort_children(
        &mut self,
        parent: NodeId,
        mut compare: impl FnMut(&Tree, NodeId, NodeId) -> std::cmp::Ordering,
    ) -> &mut Self {
        let mut children = self.children(parent).to_vec();
        // The comparator needs `&Tree`, so sort a copy and write it back.
        children.sort_by(|&a, &b| compare(self, a, b));
        if let Some(nodes) = &mut self.arena[parent.0].nodes {
            *nodes = children;
        }
        self.mark_dirty(parent);
        self
    }

    /// Removes every child of a container.
    pub fn remove_all(&mut self, parent: NodeId) -> &mut Self {
        let children = self.children(parent).to_vec();
        for child in children {
            self.arena[child.0].parent = None;
        }
        if let Some(nodes) = &mut self.arena[parent.0].nodes {
            nodes.clear();
        }
        self.mark_dirty(parent);
        self
    }

    /// Clears `before`/`after` (and `between`) on a node and its subtree.
    pub fn clean_raws(&mut self, id: NodeId, keep_between: bool) {
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            let raws = &mut self.arena[next.0].raws;
            raws.before = None;
            raws.after = None;
            if !keep_between {
                raws.between = None;
            }
            if let Some(children) = &self.arena[next.0].nodes {
                stack.extend(children.iter().copied());
            }
        }
    }

    // --- Iteration -------------------------------------------------------

    fn get_iterator(&mut self, id: NodeId) -> u32 {
        let node = &mut self.arena[id.0];
        node.last_each += 1;
        let iterator = node.last_each;
        node.set_index_slot(iterator, 0);
        iterator
    }

    /// Iterates direct children.
    ///
    /// Returns `false` when the callback broke out early. Inserting or removing
    /// nodes during the iteration shifts the cursor, so every remaining child is
    /// still visited exactly once.
    pub fn each<R: IntoVisit>(
        &mut self,
        id: NodeId,
        mut callback: impl FnMut(&mut Tree, NodeId, usize) -> R,
    ) -> bool {
        if !self.is_container(id) {
            return true;
        }
        let iterator = self.get_iterator(id);
        let mut completed = true;

        while let Some(index) = self.arena[id.0].index_slot(iterator) {
            // A negative cursor means the child it pointed at was removed; the
            // `+= 1` below brings it back into range.
            let Some(&child) = usize::try_from(index)
                .ok()
                .and_then(|index| self.children(id).get(index))
            else {
                if index < 0 {
                    self.arena[id.0].bump_index_slot(iterator, 1);
                    continue;
                }
                break;
            };
            if callback(self, child, index as usize).into_visit() == Visit::Break {
                completed = false;
                break;
            }
            self.arena[id.0].bump_index_slot(iterator, 1);
        }

        self.arena[id.0].drop_index_slot(iterator);
        completed
    }

    /// Iterates a node's whole subtree, depth first.
    ///
    /// Returns `false` when the callback broke out early.
    pub fn walk<R: IntoVisit>(
        &mut self,
        id: NodeId,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        if !self.is_container(id) {
            return true;
        }

        // An explicit stack, so deeply nested trees cannot overflow. Each frame
        // keeps a live cursor, so mutation during the walk behaves like `each()`.
        let mut stack = vec![(id, self.get_iterator(id))];

        while let Some(&(node, iterator)) = stack.last() {
            let index = self.arena[node.0].index_slot(iterator).unwrap_or(0);
            if index < 0 {
                // The child this frame pointed at was removed mid-walk.
                self.arena[node.0].bump_index_slot(iterator, 1);
                continue;
            }
            let index = index as usize;
            let len = self.children(node).len();

            if index >= len {
                self.arena[node.0].drop_index_slot(iterator);
                stack.pop();
                if let Some(&(parent, parent_iterator)) = stack.last() {
                    // Finish the parent's step for the subtree we just left.
                    self.arena[parent.0].bump_index_slot(parent_iterator, 1);
                }
                continue;
            }

            let child = self.children(node)[index];
            if callback(self, child).into_visit() == Visit::Break {
                for (opened, opened_iterator) in stack {
                    self.arena[opened.0].drop_index_slot(opened_iterator);
                }
                return false;
            }

            if self.is_container(child) {
                let iterator = self.get_iterator(child);
                stack.push((child, iterator));
            } else {
                self.arena[node.0].bump_index_slot(iterator, 1);
            }
        }

        true
    }

    /// Walks the whole tree from the root.
    pub fn walk_all<R: IntoVisit>(&mut self, callback: impl FnMut(&mut Tree, NodeId) -> R) -> bool {
        let root = self.root;
        self.walk(root, callback)
    }

    /// Walks every declaration in the tree.
    pub fn walk_decls<R: IntoVisit>(
        &mut self,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        let root = self.root;
        self.walk(root, |tree, node| {
            if matches!(tree.kind(node), NodeKind::Decl { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks declarations with a given property.
    pub fn walk_decls_with_prop<R: IntoVisit>(
        &mut self,
        prop: &str,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        self.walk_decls(|tree, node| {
            if tree.prop(node) == Some(prop) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks every rule in the tree.
    pub fn walk_rules<R: IntoVisit>(
        &mut self,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        let root = self.root;
        self.walk(root, |tree, node| {
            if matches!(tree.kind(node), NodeKind::Rule { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks rules with a given selector.
    pub fn walk_rules_with_selector<R: IntoVisit>(
        &mut self,
        selector: &str,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        self.walk_rules(|tree, node| {
            if tree.selector(node) == Some(selector) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks every at-rule in the tree.
    pub fn walk_at_rules<R: IntoVisit>(
        &mut self,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        let root = self.root;
        self.walk(root, |tree, node| {
            if matches!(tree.kind(node), NodeKind::AtRule { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks at-rules with a given name.
    pub fn walk_at_rules_with_name<R: IntoVisit>(
        &mut self,
        name: &str,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        self.walk_at_rules(|tree, node| {
            if tree.name(node) == Some(name) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks every comment in the tree.
    pub fn walk_comments<R: IntoVisit>(
        &mut self,
        mut callback: impl FnMut(&mut Tree, NodeId) -> R,
    ) -> bool {
        let root = self.root;
        self.walk(root, |tree, node| {
            if matches!(tree.kind(node), NodeKind::Comment { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Walks a subtree without borrowing the tree mutably.
    ///
    /// Cheaper than [`Tree::walk`] and usable from read-only code such as the
    /// stringifier, at the cost of not tolerating mutation mid-walk.
    pub fn walk_ref<R: IntoVisit>(
        &self,
        id: NodeId,
        mut callback: impl FnMut(&Tree, NodeId) -> R,
    ) -> bool {
        let mut stack: Vec<(NodeId, usize)> = vec![(id, 0)];
        while let Some((node, index)) = stack.pop() {
            let Some(&child) = self.children(node).get(index) else {
                continue;
            };
            stack.push((node, index + 1));
            if callback(self, child).into_visit() == Visit::Break {
                return false;
            }
            if self.is_container(child) {
                stack.push((child, 0));
            }
        }
        true
    }

    /// Read-only [`Tree::walk_decls`].
    pub fn walk_decls_ref<R: IntoVisit>(
        &self,
        id: NodeId,
        mut callback: impl FnMut(&Tree, NodeId) -> R,
    ) -> bool {
        self.walk_ref(id, |tree, node| {
            if matches!(tree.kind(node), NodeKind::Decl { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// Read-only [`Tree::walk_comments`].
    pub fn walk_comments_ref<R: IntoVisit>(
        &self,
        id: NodeId,
        mut callback: impl FnMut(&Tree, NodeId) -> R,
    ) -> bool {
        self.walk_ref(id, |tree, node| {
            if matches!(tree.kind(node), NodeKind::Comment { .. }) {
                callback(tree, node).into_visit()
            } else {
                Visit::Continue
            }
        })
    }

    /// True when every child satisfies the predicate.
    pub fn every(&self, id: NodeId, mut predicate: impl FnMut(&Tree, NodeId) -> bool) -> bool {
        self.children(id)
            .to_vec()
            .into_iter()
            .all(|child| predicate(self, child))
    }

    /// True when any child satisfies the predicate.
    pub fn some(&self, id: NodeId, mut predicate: impl FnMut(&Tree, NodeId) -> bool) -> bool {
        self.children(id)
            .to_vec()
            .into_iter()
            .any(|child| predicate(self, child))
    }

    /// Replaces a pattern in every declaration value.
    pub fn replace_values(
        &mut self,
        pattern: &str,
        props: Option<&[&str]>,
        replacement: &str,
    ) -> &mut Self {
        self.walk_decls(|tree, decl| {
            if let Some(props) = props {
                if !props.contains(&tree.prop(decl).unwrap_or_default()) {
                    return;
                }
            }
            let Some(value) = tree.value(decl) else {
                return;
            };
            if !value.contains(pattern) {
                return;
            }
            let replaced = value.replace(pattern, replacement);
            tree.set_value(decl, replaced);
        });
        self
    }

    // --- Errors and positions -------------------------------------------

    /// Builds an error pointing at a node, for plugins to return.
    pub fn node_error(
        &self,
        id: NodeId,
        message: impl Into<String>,
        opts: &NodeErrorOptions,
    ) -> CssSyntaxError {
        let Some(source) = self.source(id) else {
            let mut error = CssSyntaxError::new(message);
            if let Some(plugin) = &opts.plugin {
                error.plugin = Some(plugin.clone());
                error.set_message();
            }
            return error;
        };

        let (start, end) = self.range_by(id, opts);
        source.input.error_range(
            message,
            Loc::LineCol {
                line: start.line,
                column: start.column,
            },
            Loc::LineCol {
                line: end.line,
                column: end.column,
            },
            opts.plugin.as_deref(),
        )
    }

    /// The node's position, narrowed by `index` or `word`.
    ///
    /// Port of `Node#positionBy()`.
    pub fn position_by(&self, id: NodeId, opts: &NodeErrorOptions) -> Position {
        let source = self.source(id).expect("node has a source");
        let start = source.start.expect("node has a start position");

        if let Some(index) = opts.index {
            return self.position_inside(id, index);
        }
        if let Some(word) = &opts.word {
            if let Some(index) = self.find_word(id, word) {
                return self.position_inside(id, index);
            }
        }
        start
    }

    /// The position `index` characters into the node.
    ///
    /// Port of `Node#positionInside()`.
    pub fn position_inside(&self, id: NodeId, index: usize) -> Position {
        let source = self.source(id).expect("node has a source");
        let start = source.start.expect("node has a start position");
        let text = source.input.document();

        let mut line = start.line;
        let mut column = start.column;
        let mut offset = start.offset;

        for character in text[start.offset..].chars().take(index) {
            if character == '\n' {
                column = 1;
                line += 1;
            } else {
                column += 1;
            }
            offset += character.len_utf8();
        }

        Position {
            line,
            column,
            offset,
        }
    }

    /// The node's start and end, narrowed by the options.
    ///
    /// Port of `Node#rangeBy()`.
    pub fn range_by(&self, id: NodeId, opts: &NodeErrorOptions) -> (Position, Position) {
        let source = self.source(id).expect("node has a source");
        let node_start = source.start.expect("node has a start position");

        let mut start = node_start;
        let mut end = match source.end {
            Some(end) => Position {
                line: end.line,
                // `source.end` is inclusive in line/column but exclusive in
                // offset.
                column: end.column + 1,
                offset: end.offset,
            },
            None => Position {
                line: start.line,
                column: start.column + 1,
                offset: start.offset + 1,
            },
        };

        if let Some(word) = &opts.word {
            if let Some(index) = self.find_word(id, word) {
                start = self.position_inside(id, index);
                end = self.position_inside(id, index + word.chars().count());
            }
        } else {
            if let Some((line, column)) = opts.start {
                start = Position {
                    line,
                    column,
                    offset: source.input.from_line_and_column(line, column),
                };
            } else if let Some(index) = opts.index {
                start = self.position_inside(id, index);
            }

            if let Some((line, column)) = opts.end {
                end = Position {
                    line,
                    column,
                    offset: source.input.from_line_and_column(line, column),
                };
            } else if let Some(end_index) = opts.end_index {
                end = self.position_inside(id, end_index);
            } else if let Some(index) = opts.index {
                end = self.position_inside(id, index + 1);
            }
        }

        if end.line < start.line || (end.line == start.line && end.column <= start.column) {
            end = Position {
                line: start.line,
                column: start.column + 1,
                offset: start.offset + 1,
            };
        }

        (start, end)
    }

    /// Character index of `word` inside the node's source text.
    fn find_word(&self, id: NodeId, word: &str) -> Option<usize> {
        let source = self.source(id)?;
        let start = source.start?;
        let end = source.end?;
        let text = source.input.document();
        let slice = text.get(start.offset..end.offset)?;
        let byte_index = slice.find(word)?;
        Some(slice[..byte_index].chars().count())
    }

    // --- Stringification -------------------------------------------------

    /// Renders the whole tree back to CSS.
    pub fn to_css(&self) -> String {
        stringify_to_string(self, self.root)
    }

    /// Renders one node back to CSS.
    pub fn node_to_css(&self, id: NodeId) -> String {
        stringify_to_string(self, id)
    }

    /// The source text of a value-like property (`value`, `params`,
    /// `selector`), or the plain value once the two have diverged.
    pub fn raw_value(&self, id: NodeId, prop: &str) -> String {
        Stringifier::new(self).raw_value(id, prop)
    }

    /// Reads a raw with the stringifier's inference, used by `Node#raw()`.
    pub fn raw(&self, id: NodeId, own: Option<&str>, detect: Option<&str>) -> String {
        let mut stringifier = Stringifier::new(self);
        stringifier.raw(id, own, detect)
    }

    /// Sets `source.input` for a subtree. Used by the parser.
    pub(crate) fn set_source(&mut self, id: NodeId, source: Source) {
        self.arena[id.0].source = Some(source);
    }

    pub(crate) fn source_mut(&mut self, id: NodeId) -> Option<&mut Source> {
        self.arena[id.0].source.as_mut()
    }

    /// Every distinct [`Input`] referenced by the tree, in walk order.
    pub fn inputs(&self) -> Vec<Arc<Input>> {
        let mut result: Vec<Arc<Input>> = Vec::new();
        let mut stack = vec![self.root];
        let mut ordered = Vec::new();
        // Depth-first, children in order.
        while let Some(id) = stack.pop() {
            ordered.push(id);
            if let Some(children) = &self.arena[id.0].nodes {
                for &child in children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        for id in ordered {
            if let Some(source) = &self.arena[id.0].source {
                if !result.iter().any(|input| Arc::ptr_eq(input, &source.input)) {
                    result.push(Arc::clone(&source.input));
                }
            }
        }
        result
    }

    pub(crate) fn set_raw_value(&mut self, id: NodeId, key: &str, value: RawValue) {
        self.arena[id.0].raws.set_raw_value(key, value);
    }
}

impl Default for Tree {
    fn default() -> Self {
        Tree::new()
    }
}

impl std::fmt::Display for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_css())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertKind {
    Prepend,
}

/// Which side of the target the new nodes go on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

/// The existing child an insertion is positioned against.
#[derive(Clone, Copy, Debug)]
enum Target {
    Node(NodeId),
    Index(usize),
}

impl Target {
    fn resolve(self, tree: &Tree, parent: NodeId) -> usize {
        match self {
            Target::Node(id) => tree.index(parent, id).unwrap_or(0),
            Target::Index(index) => index,
        }
    }
}

/// Flattens one level of [`Insertable::Many`], so each item is normalized on its
/// own the way `append(...children)` does in JS.
fn split_top_level(insert: Insertable) -> Vec<Insertable> {
    match insert {
        Insertable::Many(items) => items,
        other => vec![other],
    }
}

/// `value.replace(/\S/g, '')`
fn strip_non_space(value: &str) -> String {
    value.chars().filter(|c| c.is_whitespace()).collect()
}

/// The `/,\s*/` separator already used in a selector.
fn find_comma_separator(selector: &str) -> Option<String> {
    let index = selector.find(',')?;
    let rest = &selector[index + 1..];
    let spaces: String = rest.chars().take_while(|c| c.is_whitespace()).collect();
    Some(format!(",{}", spaces))
}
