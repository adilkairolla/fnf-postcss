//! Turning the AST back into CSS.
//!
//! Port of `lib/stringifier.js`. Output is byte-identical to the input for any
//! unmodified tree; for modified or hand-built trees, missing whitespace is
//! inferred from the rest of the document, so new nodes match the file's
//! existing style.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::node::{NodeId, NodeKind};
use crate::tree::{Tree, Visit};

/// Which end of a node a chunk of output belongs to.
///
/// Source map generation needs this to map the `{` of a rule to the rule's
/// start and its `}` to the rule's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The chunk opens the node.
    Start,
    /// The chunk closes the node.
    End,
}

/// Receives the output chunks of a stringification pass.
pub trait Build {
    /// Receives one chunk of output, with the node it belongs to.
    fn push(&mut self, text: &str, node: Option<NodeId>, part: Option<Part>);
}

impl<F> Build for F
where
    F: FnMut(&str, Option<NodeId>, Option<Part>),
{
    fn push(&mut self, text: &str, node: Option<NodeId>, part: Option<Part>) {
        self(text, node, part)
    }
}

/// A builder that throws the output away, for callers that only want
/// [`Stringifier::raw`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopBuilder;

impl Build for NoopBuilder {
    fn push(&mut self, _text: &str, _node: Option<NodeId>, _part: Option<Part>) {}
}

/// A builder that concatenates into a `String`.
#[derive(Clone, Debug, Default)]
pub struct StringBuilder {
    /// The output collected so far.
    pub css: String,
}

impl Build for StringBuilder {
    fn push(&mut self, text: &str, _node: Option<NodeId>, _part: Option<Part>) {
        self.css.push_str(text);
    }
}

/// Renders a node to a string.
pub fn stringify_to_string(tree: &Tree, id: NodeId) -> String {
    let mut builder = StringBuilder::default();
    stringify(tree, id, &mut builder);
    builder.css
}

/// Renders a node, handing every chunk to `builder`.
pub fn stringify(tree: &Tree, id: NodeId, builder: &mut dyn Build) {
    let mut stringifier = Stringifier::with_builder(tree, builder);
    stringifier.stringify_node(id, false);
}

fn default_raw(detect: &str) -> &'static str {
    match detect {
        "after" => "\n",
        "beforeClose" => "\n",
        "beforeComment" => "\n",
        "beforeDecl" => "\n",
        "beforeOpen" => " ",
        "beforeRule" => "\n",
        "colon" => ": ",
        "commentLeft" => " ",
        "commentRight" => " ",
        "emptyBody" => "",
        "indent" => "    ",
        _ => "",
    }
}

/// Characters that end an at-rule name, mirroring `RE_AT_END` in the tokenizer.
/// Params starting with anything else need a space to stay separate tokens.
fn is_at_name_end(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{c}' | '\r' | ' ' | '"' | '#' | '\'' | '(' | ')' | '/' | ';' | '[' | '\\'
            | ']' | '{' | '}'
    )
}

/// Escapes sequences that could break out of an HTML `<style>` context.
///
/// Uses CSS unicode escaping (`\3c` = `<`), which is valid CSS and parsed
/// correctly by all compliant CSS consumers.
pub fn escape_html_in_css(text: &str) -> Cow<'_, str> {
    // The overwhelming majority of chunks contain no `<` at all, so the fast
    // path must not allocate.
    if !text.contains('<') {
        return Cow::Borrowed(text);
    }

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &text[i + 1..];
            let tag = rest.strip_prefix('/').unwrap_or(rest);
            // `/(<)(\/?style\b)/gi`: `\b` means the name must not run on.
            let is_style = tag
                .get(..5)
                .is_some_and(|name| name.eq_ignore_ascii_case("style"))
                && !tag[5..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if is_style || rest.starts_with("!--") {
                out.push_str("\\3c ");
                i += 1;
                continue;
            }
        }
        let character = text[i..].chars().next().expect("valid UTF-8 boundary");
        out.push(character);
        i += character.len_utf8();
    }

    Cow::Owned(out)
}

/// One stringification pass.
pub struct Stringifier<'t, 'b> {
    tree: &'t Tree,
    /// `None` when the caller only wants raw lookups.
    builder: Option<&'b mut dyn Build>,
    /// Style inferred from the document, cached per pass — the JS version caches
    /// this on the root, where it can go stale between calls.
    cache: HashMap<String, String>,
    semicolon_cache: Option<bool>,
}

/// A frame of the explicit output stack.
enum Frame {
    /// Emit a child node.
    Node {
        node: NodeId,
        semicolon: bool,
        document: bool,
    },
    /// Close a block whose children have all been emitted.
    Close { node: NodeId, has_nodes: bool },
}

impl<'t> Stringifier<'t, '_> {
    /// A stringifier that produces no output, for raw lookups only.
    pub fn new(tree: &'t Tree) -> Self {
        Stringifier {
            tree,
            builder: None,
            cache: HashMap::new(),
            semicolon_cache: None,
        }
    }
}

impl<'t, 'b> Stringifier<'t, 'b> {
    /// A stringifier writing to `builder`.
    pub fn with_builder(tree: &'t Tree, builder: &'b mut dyn Build) -> Self {
        Stringifier {
            tree,
            builder: Some(builder),
            cache: HashMap::new(),
            semicolon_cache: None,
        }
    }

    fn emit(&mut self, text: &str, node: Option<NodeId>, part: Option<Part>) {
        if let Some(builder) = &mut self.builder {
            builder.push(text, node, part);
        }
    }

    /// Renders a node, appending `;` when `semicolon` is set.
    pub fn stringify_node(&mut self, id: NodeId, semicolon: bool) {
        match self.tree.kind(id).clone() {
            NodeKind::Root => self.root(id),
            NodeKind::Document => self.body(id),
            NodeKind::Rule { .. } => self.rule(id),
            NodeKind::AtRule { .. } => self.atrule(id, semicolon),
            NodeKind::Decl { .. } => self.decl(id, semicolon),
            NodeKind::Comment { .. } => self.comment(id),
        }
    }

    fn root(&mut self, id: NodeId) {
        self.body(id);
        if let Some(after) = self.tree.raws(id).after.clone() {
            if !after.is_empty() {
                let is_document = self
                    .tree
                    .parent(id)
                    .is_some_and(|parent| self.tree.type_name(parent) == "document");
                if is_document {
                    self.emit(&after, None, None);
                } else {
                    let text = escape_html_in_css(&after).into_owned();
                    self.emit(&text, None, None);
                }
            }
        }
    }

    fn rule(&mut self, id: NodeId) {
        let start = self.raw_value(id, "selector");
        self.block(id, &start);
        if let Some(own_semicolon) = self.tree.raws(id).own_semicolon.clone() {
            let text = escape_html_in_css(&own_semicolon);
            self.emit(&text, Some(id), Some(Part::End));
        }
    }

    fn atrule(&mut self, id: NodeId, semicolon: bool) {
        let start = self.atrule_start(id);
        if self.tree.is_container(id) {
            self.block(id, &start);
        } else {
            let between = self.tree.raws(id).between.clone().unwrap_or_default();
            let end = if semicolon { ";" } else { "" };
            let raw = format!("{}{}{}", start, between, end);
            let text = escape_html_in_css(&raw).into_owned();
            self.emit(&text, Some(id), None);
        }
    }

    fn decl(&mut self, id: NodeId, semicolon: bool) {
        let between = self.raw(id, Some("between"), Some("colon"));
        let prop = self.tree.prop(id).unwrap_or_default().to_string();
        let value = self.raw_value(id, "value");

        let mut string = format!("{}{}{}", prop, between, value);
        if self.tree.important(id) {
            string.push_str(
                self.tree
                    .raws(id)
                    .important
                    .as_deref()
                    .unwrap_or(" !important"),
            );
        }
        if semicolon {
            string.push(';');
        }

        let text = escape_html_in_css(&string);
        self.emit(&text, Some(id), None);
    }

    fn comment(&mut self, id: NodeId) {
        let left = self.raw(id, Some("left"), Some("commentLeft"));
        let right = self.raw(id, Some("right"), Some("commentRight"));
        let text = self.tree.text(id).unwrap_or_default().to_string();
        let raw = format!("/*{}{}{}*/", left, text, right);
        let text = escape_html_in_css(&raw).into_owned();
        self.emit(&text, Some(id), None);
    }

    /// `@name` plus params, with the separator the source used.
    fn atrule_start(&mut self, id: NodeId) -> String {
        let name = format!("@{}", self.tree.name(id).unwrap_or_default());
        let params = if self.tree.params(id).is_some_and(|p| !p.is_empty()) {
            self.raw_value(id, "params")
        } else {
            String::new()
        };

        let after_name = match self.tree.raws(id).after_name.clone() {
            None => {
                if params.is_empty() {
                    String::new()
                } else {
                    " ".to_string()
                }
            }
            Some(after_name) => {
                let needs_space = after_name.is_empty()
                    && !params.is_empty()
                    && !params.chars().next().is_some_and(is_at_name_end);
                if needs_space {
                    " ".to_string()
                } else {
                    after_name
                }
            }
        };

        format!("{}{}{}", name, after_name, params)
    }

    fn block(&mut self, id: NodeId, start: &str) {
        let between = self.raw(id, Some("between"), Some("beforeOpen"));
        let opening = format!("{}{{", escape_html_in_css(&format!("{}{}", start, between)));
        self.emit(&opening, Some(id), Some(Part::Start));

        let has_nodes = !self.tree.children(id).is_empty();
        if has_nodes {
            self.body(id);
        }
        self.close(id, has_nodes);
    }

    fn close(&mut self, id: NodeId, has_nodes: bool) {
        let after = if has_nodes {
            self.raw(id, Some("after"), None)
        } else {
            self.raw(id, Some("after"), Some("emptyBody"))
        };
        if !after.is_empty() {
            let text = escape_html_in_css(&after);
            self.emit(&text, None, None);
        }
        self.emit("}", Some(id), Some(Part::End));
    }

    /// Emits a container's children.
    ///
    /// Uses an explicit stack rather than recursion, so deeply nested trees
    /// cannot overflow.
    fn body(&mut self, id: NodeId) {
        let mut stack: Vec<Frame> = Vec::new();
        self.push_body(&mut stack, id);

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Close { node, has_nodes } => {
                    self.close(node, has_nodes);
                    if self.tree.type_name(node) == "rule" {
                        if let Some(own_semicolon) = self.tree.raws(node).own_semicolon.clone() {
                            let text = escape_html_in_css(&own_semicolon);
                            self.emit(&text, Some(node), Some(Part::End));
                        }
                    }
                }
                Frame::Node {
                    node,
                    semicolon,
                    document,
                } => {
                    let before = self.raw(node, Some("before"), None);
                    if !before.is_empty() {
                        if document {
                            self.emit(&before, None, None);
                        } else {
                            let text = escape_html_in_css(&before).into_owned();
                            self.emit(&text, None, None);
                        }
                    }

                    match self.tree.kind(node) {
                        NodeKind::Rule { .. } => {
                            let start = self.raw_value(node, "selector");
                            self.push_block(&mut stack, node, &start);
                        }
                        NodeKind::AtRule { .. } if self.tree.is_container(node) => {
                            let start = self.atrule_start(node);
                            self.push_block(&mut stack, node, &start);
                        }
                        _ => self.stringify_node(node, semicolon),
                    }
                }
            }
        }
    }

    fn push_body(&mut self, stack: &mut Vec<Frame>, id: NodeId) {
        let nodes = self.tree.children(id).to_vec();
        if nodes.is_empty() {
            return;
        }

        // Trailing comments do not need the separator the last declaration does.
        let mut last = nodes.len() - 1;
        while last > 0 {
            if self.tree.type_name(nodes[last]) != "comment" {
                break;
            }
            last -= 1;
        }

        let semicolon = self.raw_semicolon(id);
        let is_document = self.tree.type_name(id) == "document";

        for (index, &child) in nodes.iter().enumerate().rev() {
            let mut child_semicolon = last != index || semicolon;

            // A childless at-rule or a custom property declaration that still
            // has following siblings must be terminated. Without the semicolon
            // those trailing comments are folded into the at-rule's prelude or
            // the custom property's value and disappear when the output is
            // re-parsed.
            if !child_semicolon && index < nodes.len() - 1 {
                let needs_semicolon = match self.tree.kind(child) {
                    NodeKind::AtRule { .. } => !self.tree.is_container(child),
                    NodeKind::Decl { prop, .. } => prop.starts_with("--"),
                    _ => false,
                };
                if needs_semicolon {
                    child_semicolon = true;
                }
            }

            stack.push(Frame::Node {
                node: child,
                semicolon: child_semicolon,
                document: is_document,
            });
        }
    }

    fn push_block(&mut self, stack: &mut Vec<Frame>, id: NodeId, start: &str) {
        let between = self.raw(id, Some("between"), Some("beforeOpen"));
        let opening = format!("{}{{", escape_html_in_css(&format!("{}{}", start, between)));
        self.emit(&opening, Some(id), Some(Part::Start));

        let has_nodes = !self.tree.children(id).is_empty();
        if has_nodes {
            stack.push(Frame::Close {
                node: id,
                has_nodes,
            });
            self.push_body(stack, id);
        } else {
            self.close(id, has_nodes);
            if self.tree.type_name(id) == "rule" {
                if let Some(own_semicolon) = self.tree.raws(id).own_semicolon.clone() {
                    let text = escape_html_in_css(&own_semicolon);
                    self.emit(&text, Some(id), Some(Part::End));
                }
            }
        }
    }

    /// The source text of a value-like property, or the value itself once the
    /// two have diverged.
    pub fn raw_value(&self, id: NodeId, prop: &str) -> String {
        let value = match prop {
            "value" => self.tree.value(id).unwrap_or_default(),
            "params" => self.tree.params(id).unwrap_or_default(),
            "selector" => self.tree.selector(id).unwrap_or_default(),
            _ => "",
        };

        match self.tree.raws(id).raw_value(prop) {
            Some(raw) if raw.value == value => raw.raw.clone(),
            _ => value.to_string(),
        }
    }

    /// Reads a raw, inferring it from the rest of the document when the node
    /// does not carry one.
    ///
    /// Port of `Stringifier#raw()`.
    pub fn raw(&mut self, id: NodeId, own: Option<&str>, detect: Option<&str>) -> String {
        let detect = detect.or(own).unwrap_or("");

        // Already had
        if let Some(own) = own {
            if let Some(value) = self.tree.raws(id).get_str(own) {
                return value.to_string();
            }
        }

        let parent = self.tree.parent(id);

        if detect == "before" {
            // Hack for first rule in CSS
            match parent {
                None => return String::new(),
                Some(parent) => {
                    if self.tree.type_name(parent) == "root" && self.tree.first(parent) == Some(id) {
                        return String::new();
                    }
                    // `root` nodes in `document` should use only their own raws
                    if self.tree.type_name(parent) == "document" {
                        return String::new();
                    }
                }
            }
        }

        // Floating child without parent
        if parent.is_none() {
            return default_raw(detect).to_string();
        }

        let root = self.tree.root_of(id);
        if let Some(cached) = self.cache.get(detect) {
            return cached.clone();
        }

        if detect == "before" || detect == "after" {
            return self.before_after(id, detect);
        }

        let value = match detect {
            "beforeClose" => self.raw_before_close(root),
            "beforeComment" => Some(self.raw_before_comment(root, id)),
            "beforeDecl" => Some(self.raw_before_decl(root, id)),
            "beforeOpen" => self.raw_before_open(root),
            "beforeRule" => self.raw_before_rule(root),
            "colon" => self.raw_colon(root),
            "emptyBody" => self.raw_empty_body(root),
            "indent" => self.raw_indent(root),
            _ => own.and_then(|own| self.raw_from_any_node(root, own)),
        };

        let value = value.unwrap_or_else(|| default_raw(detect).to_string());
        self.cache.insert(detect.to_string(), value.clone());
        value
    }

    /// `raws.semicolon`, inferred the same way as the string raws.
    fn raw_semicolon(&mut self, id: NodeId) -> bool {
        if let Some(semicolon) = self.tree.raws(id).semicolon {
            return semicolon;
        }
        if self.tree.parent(id).is_none() {
            return false;
        }
        if let Some(cached) = self.semicolon_cache {
            return cached;
        }

        let root = self.tree.root_of(id);
        let mut found = None;
        self.tree.walk_ref(root, |tree, node| {
            let children = tree.children(node);
            if !children.is_empty() && tree.type_name(*children.last().unwrap()) == "decl" {
                if let Some(semicolon) = tree.raws(node).semicolon {
                    found = Some(semicolon);
                    return Visit::Break;
                }
            }
            Visit::Continue
        });

        let value = found.unwrap_or(false);
        self.semicolon_cache = Some(value);
        value
    }

    /// Indentation-aware `before`/`after`.
    ///
    /// Port of `Stringifier#beforeAfter()`.
    fn before_after(&mut self, id: NodeId, detect: &str) -> String {
        let mut value = match self.tree.kind(id) {
            NodeKind::Decl { .. } => self.raw(id, None, Some("beforeDecl")),
            NodeKind::Comment { .. } => self.raw(id, None, Some("beforeComment")),
            _ if detect == "before" => self.raw(id, None, Some("beforeRule")),
            _ => self.raw(id, None, Some("beforeClose")),
        };

        let mut depth = 0;
        let mut buf = self.tree.parent(id);
        while let Some(node) = buf {
            if self.tree.type_name(node) == "root" {
                break;
            }
            depth += 1;
            buf = self.tree.parent(node);
        }

        if value.contains('\n') {
            let indent = self.raw(id, None, Some("indent"));
            if !indent.is_empty() {
                for _ in 0..depth {
                    value.push_str(&indent);
                }
            }
        }

        value
    }

    fn raw_before_close(&self, root: NodeId) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            if !tree.children(node).is_empty() {
                if let Some(after) = &tree.raws(node).after {
                    value = Some(keep_up_to_last_newline(after));
                    return Visit::Break;
                }
            }
            Visit::Continue
        });
        value.map(|value| {
            if value.is_empty() {
                value
            } else {
                strip_non_space(&value)
            }
        })
    }

    fn raw_before_comment(&mut self, root: NodeId, id: NodeId) -> String {
        let mut value: Option<String> = None;
        self.tree.walk_comments_ref(root, |tree, node| {
            if let Some(before) = &tree.raws(node).before {
                value = Some(keep_up_to_last_newline(before));
                return Visit::Break;
            }
            Visit::Continue
        });

        match value {
            None => self.raw(id, None, Some("beforeDecl")),
            Some(value) if value.is_empty() => value,
            Some(value) => strip_non_space(&value),
        }
    }

    fn raw_before_decl(&mut self, root: NodeId, id: NodeId) -> String {
        let mut value: Option<String> = None;
        self.tree.walk_decls_ref(root, |tree, node| {
            if let Some(before) = &tree.raws(node).before {
                value = Some(keep_up_to_last_newline(before));
                return Visit::Break;
            }
            Visit::Continue
        });

        match value {
            None => self.raw(id, None, Some("beforeRule")),
            Some(value) if value.is_empty() => value,
            Some(value) => strip_non_space(&value),
        }
    }

    fn raw_before_open(&self, root: NodeId) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            if tree.type_name(node) != "decl" {
                if let Some(between) = &tree.raws(node).between {
                    value = Some(between.clone());
                    return Visit::Break;
                }
            }
            Visit::Continue
        });
        value
    }

    fn raw_before_rule(&self, root: NodeId) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            let is_nested = tree.parent(node) != Some(root) || tree.first(root) != Some(node);
            if tree.is_container(node) && is_nested {
                if let Some(before) = &tree.raws(node).before {
                    value = Some(keep_up_to_last_newline(before));
                    return Visit::Break;
                }
            }
            Visit::Continue
        });
        value.map(|value| {
            if value.is_empty() {
                value
            } else {
                strip_non_space(&value)
            }
        })
    }

    fn raw_colon(&self, root: NodeId) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_decls_ref(root, |tree, node| {
            if let Some(between) = &tree.raws(node).between {
                value = Some(
                    between
                        .chars()
                        .filter(|c| c.is_whitespace() || *c == ':')
                        .collect(),
                );
                return Visit::Break;
            }
            Visit::Continue
        });
        value
    }

    fn raw_empty_body(&self, root: NodeId) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            if tree.is_container(node) && tree.children(node).is_empty() {
                if let Some(after) = &tree.raws(node).after {
                    value = Some(after.clone());
                    return Visit::Break;
                }
            }
            Visit::Continue
        });
        value
    }

    fn raw_indent(&self, root: NodeId) -> Option<String> {
        if let Some(indent) = &self.tree.raws(root).indent {
            return Some(indent.clone());
        }

        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            let parent = tree.parent(node);
            let is_grandchild = parent.is_some_and(|parent| {
                parent != root && tree.parent(parent).is_some_and(|grand| grand == root)
            });
            if is_grandchild {
                if let Some(before) = &tree.raws(node).before {
                    let last_line = before.rsplit('\n').next().unwrap_or("");
                    value = Some(strip_non_space(last_line));
                    return Visit::Break;
                }
            }
            Visit::Continue
        });
        value
    }

    /// Fallback for raws with no dedicated detector: the first value any node in
    /// the tree carries.
    fn raw_from_any_node(&self, root: NodeId, own: &str) -> Option<String> {
        let mut value: Option<String> = None;
        self.tree.walk_ref(root, |tree, node| {
            if let Some(found) = tree.raws(node).get_str(own) {
                value = Some(found.to_string());
                return Visit::Break;
            }
            Visit::Continue
        });
        value
    }
}

/// `value.replace(/[^\n]+$/, '')` — drops the trailing indentation of a line.
fn keep_up_to_last_newline(value: &str) -> String {
    match value.rfind('\n') {
        Some(index) => value[..index + 1].to_string(),
        None => value.to_string(),
    }
}

/// `value.replace(/\S/g, '')`
fn strip_non_space(value: &str) -> String {
    value.chars().filter(|c| c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_style_tags() {
        assert_eq!(escape_html_in_css("a{}"), "a{}");
        assert_eq!(escape_html_in_css("a < b"), "a < b");
        assert_eq!(escape_html_in_css("</style>"), "\\3c /style>");
        assert_eq!(escape_html_in_css("<style>"), "\\3c style>");
        assert_eq!(escape_html_in_css("<STYLE "), "\\3c STYLE ");
        assert_eq!(escape_html_in_css("<styles>"), "<styles>");
        assert_eq!(escape_html_in_css("<!--"), "\\3c !--");
    }
}
