//! The CSS parser.
//!
//! Port of `lib/parser.js`. Every `raws` field and source offset is produced the
//! same way as in JS, which is what lets an unmodified tree stringify back to
//! byte-identical CSS.

use std::sync::Arc;

use crate::error::CssSyntaxError;
use crate::input::{Input, Loc};
use crate::node::{NewNode, NodeId, NodeKind, Position, RawValue, Source};
use crate::tokenize::{Token, TokenKind, Tokenizer, TokenizerOptions};
use crate::tree::Tree;

/// Comment neighbours that let a comment be dropped from a cleaned value.
fn is_safe_comment_neighbor(kind: Option<TokenKind>) -> bool {
    match kind {
        // `empty`, i.e. no neighbour at all.
        None => true,
        Some(kind) => kind == TokenKind::Space,
    }
}

fn find_last_with_position(tokens: &[Token<'_>]) -> Option<usize> {
    tokens.iter().rev().find_map(|token| token.truthy_position())
}

fn tokens_to_string(tokens: &[Token<'_>], from: usize, to: usize) -> String {
    let mut result = String::new();
    for token in &tokens[from..to.min(tokens.len())] {
        result.push_str(token.content);
    }
    result
}

/// Turns tokens into a [`Tree`].
pub struct Parser<'a> {
    input: &'a Arc<Input>,
    tree: Tree,
    root: NodeId,
    current: NodeId,
    spaces: String,
    semicolon: bool,
    tokenizer: Tokenizer<'a>,
}

impl<'a> Parser<'a> {
    /// Creates a parser reading `input`.
    pub fn new(input: &'a Arc<Input>) -> Self {
        let tree = Tree::new();
        let root = tree.root();
        let mut parser = Parser {
            input,
            tree,
            root,
            current: root,
            spaces: String::new(),
            semicolon: false,
            tokenizer: Tokenizer::new(input, TokenizerOptions::default()),
        };
        parser.tree.set_source(
            root,
            Source {
                input: Arc::clone(input),
                start: Some(Position {
                    line: 1,
                    column: 1,
                    offset: 0,
                }),
                end: None,
            },
        );
        parser
    }

    /// Parses the whole input.
    pub fn parse(mut self) -> Result<Tree, CssSyntaxError> {
        while !self.tokenizer.end_of_file() {
            let Some(token) = self.tokenizer.next_token(false)? else {
                break;
            };

            match token.kind {
                TokenKind::Space => self.spaces.push_str(token.content),
                TokenKind::Semicolon => self.free_semicolon(&token),
                TokenKind::CloseCurly => self.end(&token)?,
                TokenKind::Comment => self.comment(&token),
                TokenKind::AtWord => self.atrule(&token)?,
                TokenKind::OpenCurly => self.empty_rule(&token),
                _ => self.other(token)?,
            }
        }
        self.end_file()?;
        Ok(self.tree)
    }

    // --- Node builders ---------------------------------------------------

    fn atrule(&mut self, token: &Token<'a>) -> Result<(), CssSyntaxError> {
        let name = &token.content[1..];
        if name.is_empty() {
            return Err(self.unnamed_atrule(token));
        }

        let node = self.tree.create(NewNode::at_rule(name, ""));
        self.init(node, token.start_or_zero());

        let mut last = false;
        let mut open = false;
        let mut params: Vec<Token<'a>> = Vec::new();
        let mut brackets: Vec<TokenKind> = Vec::new();

        while !self.tokenizer.end_of_file() {
            let Some(token) = self.tokenizer.next_token(false)? else {
                break;
            };
            let kind = token.kind;

            if kind == TokenKind::OpenParen || kind == TokenKind::OpenSquare {
                brackets.push(if kind == TokenKind::OpenParen {
                    TokenKind::CloseParen
                } else {
                    TokenKind::CloseSquare
                });
            } else if kind == TokenKind::OpenCurly && !brackets.is_empty() {
                brackets.push(TokenKind::CloseCurly);
            } else if brackets.last() == Some(&kind) {
                brackets.pop();
            }

            if brackets.is_empty() {
                if kind == TokenKind::Semicolon {
                    let mut end = self.get_position(token.start_or_zero());
                    end.offset += 1;
                    self.set_end(node, end);
                    self.semicolon = true;
                    break;
                } else if kind == TokenKind::OpenCurly {
                    open = true;
                    break;
                } else if kind == TokenKind::CloseCurly {
                    if !params.is_empty() {
                        let mut shift = params.len() - 1;
                        let mut prev = Some(params[shift]);
                        while prev.is_some_and(|token| token.kind == TokenKind::Space) {
                            if shift == 0 {
                                prev = None;
                                break;
                            }
                            shift -= 1;
                            prev = Some(params[shift]);
                        }
                        if let Some(prev) = prev {
                            let mut end =
                                self.get_position(prev.end_or_start().unwrap_or_default());
                            end.offset += 1;
                            self.set_end(node, end);
                        }
                    }
                    self.end(&token)?;
                    break;
                } else {
                    params.push(token);
                }
            } else {
                params.push(token);
            }

            if self.tokenizer.end_of_file() {
                last = true;
                break;
            }
        }

        let between = spaces_and_comments_from_end(&mut params);
        self.tree.raws_mut(node).between = Some(between.clone());

        if !params.is_empty() {
            let after_name = spaces_and_comments_from_start(&mut params);
            self.tree.raws_mut(node).after_name = Some(after_name);
            self.raw(node, "params", &params, false);
            if last {
                let token = params[params.len() - 1];
                let mut end = self.get_position(token.end_or_start().unwrap_or_default());
                end.offset += 1;
                self.set_end(node, end);
                self.spaces = between;
                self.tree.raws_mut(node).between = Some(String::new());
            }
        } else {
            self.tree.raws_mut(node).after_name = Some(String::new());
            self.tree.set_params(node, "");
        }

        if open {
            self.tree.make_container(node);
            self.current = node;
        }

        Ok(())
    }

    fn comment(&mut self, token: &Token<'a>) {
        let node = self.tree.create(NewNode::comment(""));
        self.init(node, token.start_or_zero());
        let mut end = self.get_position(token.end_or_start().unwrap_or_default());
        end.offset += 1;
        self.set_end(node, end);

        // An unclosed comment can be shorter than its delimiters when the
        // tokenizer was told to ignore errors.
        let text = token
            .content
            .len()
            .checked_sub(4)
            .and_then(|_| token.content.get(2..token.content.len() - 2))
            .unwrap_or("");

        if text.trim().is_empty() {
            self.tree.set_text(node, "");
            let raws = self.tree.raws_mut(node);
            raws.left = Some(text.to_string());
            raws.right = Some(String::new());
        } else {
            // `/^(\s*)([^]*\S)(\s*)$/`
            let start = text
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(text.len());
            let end_index = text
                .rfind(|c: char| !c.is_whitespace())
                .map(|index| index + text[index..].chars().next().map_or(1, char::len_utf8))
                .unwrap_or(text.len());
            self.tree.set_text(node, &text[start..end_index]);
            let raws = self.tree.raws_mut(node);
            raws.left = Some(text[..start].to_string());
            raws.right = Some(text[end_index..].to_string());
        }
    }

    fn empty_rule(&mut self, token: &Token<'a>) {
        let node = self.tree.create(NewNode::rule(""));
        self.init(node, token.start_or_zero());
        self.tree.raws_mut(node).between = Some(String::new());
        self.current = node;
    }

    fn rule(&mut self, mut tokens: Vec<Token<'a>>) -> Result<(), CssSyntaxError> {
        tokens.pop();

        let node = self.tree.create(NewNode::rule(""));
        self.init(node, tokens[0].start_or_zero());

        let between = spaces_and_comments_from_end(&mut tokens);
        self.tree.raws_mut(node).between = Some(between);
        self.raw(node, "selector", &tokens, false);
        self.current = node;
        Ok(())
    }

    fn decl(
        &mut self,
        mut tokens: Vec<Token<'a>>,
        custom_property: bool,
    ) -> Result<(), CssSyntaxError> {
        let node = self.tree.create(NewNode::decl("", ""));
        self.init(node, tokens[0].start_or_zero());

        let last = tokens[tokens.len() - 1];
        if last.kind == TokenKind::Semicolon {
            self.semicolon = true;
            tokens.pop();
        }

        let end_offset = last
            .truthy_position()
            .or_else(|| find_last_with_position(&tokens))
            .unwrap_or(0);
        let mut end = self.get_position(end_offset);
        end.offset += 1;
        self.set_end(node, end);

        let mut start = 0;
        while tokens[start].kind != TokenKind::Word {
            if start == tokens.len() - 1 {
                return Err(self.unknown_word(&tokens[start..start + 1]));
            }
            start += 1;
        }

        let before_extra = tokens_to_string(&tokens, 0, start);
        if let Some(before) = &mut self.tree.raws_mut(node).before {
            before.push_str(&before_extra);
        }
        let node_start = self.get_position(tokens[start].start_or_zero());
        self.set_start(node, node_start);

        let prop_start = start;
        while start < tokens.len() {
            let kind = tokens[start].kind;
            if matches!(
                kind,
                TokenKind::Colon | TokenKind::Space | TokenKind::Comment
            ) {
                break;
            }
            start += 1;
        }
        let prop = tokens_to_string(&tokens, prop_start, start);
        self.tree.set_prop(node, &prop);

        let between_start = start;
        while start < tokens.len() {
            let token = tokens[start];
            start += 1;
            if token.kind == TokenKind::Colon {
                break;
            }
            if token.kind == TokenKind::Word && token.content.contains(is_word_char) {
                return Err(self.unknown_word(&[token]));
            }
        }
        let between = tokens_to_string(&tokens, between_start, start);
        self.tree.raws_mut(node).between = Some(between);

        // `_color` and `*color` are old IE hacks: the marker belongs to the
        // whitespace before the property, not to the property itself.
        let first_char = prop.chars().next();
        if first_char == Some('_') || first_char == Some('*') {
            let marker = first_char.unwrap();
            if let Some(before) = &mut self.tree.raws_mut(node).before {
                before.push(marker);
            }
            self.tree.set_prop(node, &prop[marker.len_utf8()..]);
        }

        let first_spaces_start = start;
        while start < tokens.len() {
            let kind = tokens[start].kind;
            if kind != TokenKind::Space && kind != TokenKind::Comment {
                break;
            }
            start += 1;
        }
        let mut first_spaces: Vec<Token<'a>> = tokens[first_spaces_start..start].to_vec();
        let mut tokens: Vec<Token<'a>> = tokens[start..].to_vec();

        let mut i = tokens.len();
        while i > 0 {
            i -= 1;
            let token = tokens[i];
            let lowered = token.content.to_lowercase();

            if lowered == "!important" {
                self.tree.set_important(node, true);
                let mut string = string_from(&mut tokens, i);
                string = format!("{}{}", spaces_from_end(&mut tokens), string);
                if string != " !important" {
                    self.tree.raws_mut(node).important = Some(string);
                }
                break;
            } else if lowered == "important" {
                // `! important` with anything between the two: collect
                // backwards until the `!` shows up.
                let mut cache = tokens.clone();
                let mut string = String::new();
                let mut j = i;
                while j > 0 {
                    let kind = cache[j].kind;
                    if string.trim().starts_with('!') && kind != TokenKind::Space {
                        break;
                    }
                    let popped = cache.pop().expect("cache is not empty");
                    string = format!("{}{}", popped.content, string);
                    j -= 1;
                }
                if string.trim().starts_with('!') {
                    self.tree.set_important(node, true);
                    self.tree.raws_mut(node).important = Some(string);
                    tokens = cache;
                }
            }

            if token.kind != TokenKind::Space && token.kind != TokenKind::Comment {
                break;
            }
        }

        let has_word = tokens
            .iter()
            .any(|token| token.kind != TokenKind::Space && token.kind != TokenKind::Comment);

        if has_word {
            let extra: String = first_spaces.iter().map(|token| token.content).collect();
            if let Some(between) = &mut self.tree.raws_mut(node).between {
                between.push_str(&extra);
            }
            first_spaces = Vec::new();
        }

        let mut value_tokens = first_spaces;
        value_tokens.extend(tokens.iter().copied());
        self.raw(node, "value", &value_tokens, custom_property);

        if self.tree.value(node).is_some_and(|value| value.contains(':')) && !custom_property {
            self.check_missed_semicolon(&tokens)?;
        }

        Ok(())
    }

    /// Reads a rule, declaration or unknown construct starting at `start`.
    fn other(&mut self, start: Token<'a>) -> Result<(), CssSyntaxError> {
        let mut end = false;
        let mut colon = false;
        let mut bracket: Option<Token<'a>> = None;
        let mut brackets: Vec<TokenKind> = Vec::new();
        let custom_property = start.content.starts_with("--");

        let mut tokens: Vec<Token<'a>> = Vec::new();
        let mut token = Some(start);

        while let Some(current) = token {
            let kind = current.kind;
            tokens.push(current);

            if kind == TokenKind::OpenParen || kind == TokenKind::OpenSquare {
                if bracket.is_none() {
                    bracket = Some(current);
                }
                brackets.push(if kind == TokenKind::OpenParen {
                    TokenKind::CloseParen
                } else {
                    TokenKind::CloseSquare
                });
            } else if custom_property && colon && kind == TokenKind::OpenCurly {
                if bracket.is_none() {
                    bracket = Some(current);
                }
                brackets.push(TokenKind::CloseCurly);
            } else if brackets.is_empty() {
                if kind == TokenKind::Semicolon {
                    if colon {
                        return self.decl(tokens, custom_property);
                    }
                    break;
                } else if kind == TokenKind::OpenCurly {
                    return self.rule(tokens);
                } else if kind == TokenKind::CloseCurly {
                    if let Some(popped) = tokens.pop() {
                        self.tokenizer.back(popped);
                    }
                    end = true;
                    break;
                } else if kind == TokenKind::Colon {
                    colon = true;
                }
            } else if brackets.last() == Some(&kind) {
                brackets.pop();
                if brackets.is_empty() {
                    bracket = None;
                }
            }

            token = self.tokenizer.next_token(false)?;
        }

        if self.tokenizer.end_of_file() {
            end = true;
        }
        if !brackets.is_empty() {
            return Err(self.unclosed_bracket(&bracket.expect("bracket was recorded")));
        }

        if end && colon {
            if !custom_property {
                while !tokens.is_empty() {
                    let kind = tokens[tokens.len() - 1].kind;
                    if kind != TokenKind::Space && kind != TokenKind::Comment {
                        break;
                    }
                    if let Some(popped) = tokens.pop() {
                        self.tokenizer.back(popped);
                    }
                }
            }
            self.decl(tokens, custom_property)
        } else {
            Err(self.unknown_word(&tokens))
        }
    }

    fn end(&mut self, token: &Token<'a>) -> Result<(), CssSyntaxError> {
        if !self.tree.children(self.current).is_empty() {
            self.tree.raws_mut(self.current).semicolon = Some(self.semicolon);
        }
        self.semicolon = false;

        let spaces = std::mem::take(&mut self.spaces);
        let after = self.tree.raws(self.current).after.clone().unwrap_or_default();
        self.tree.raws_mut(self.current).after = Some(format!("{}{}", after, spaces));

        match self.tree.parent(self.current) {
            Some(parent) => {
                let mut end = self.get_position(token.start_or_zero());
                end.offset += 1;
                self.set_end(self.current, end);
                self.current = parent;
                Ok(())
            }
            None => Err(self.unexpected_close(token)),
        }
    }

    fn end_file(&mut self) -> Result<(), CssSyntaxError> {
        if self.tree.parent(self.current).is_some() {
            return Err(self.unclosed_block());
        }
        if !self.tree.children(self.current).is_empty() {
            self.tree.raws_mut(self.current).semicolon = Some(self.semicolon);
        }
        let spaces = std::mem::take(&mut self.spaces);
        let after = self.tree.raws(self.current).after.clone().unwrap_or_default();
        self.tree.raws_mut(self.current).after = Some(format!("{}{}", after, spaces));

        let end = self.get_position(self.tokenizer.position());
        let root = self.root;
        self.set_end(root, end);
        Ok(())
    }

    /// A `;` with no declaration before it belongs to the preceding rule.
    fn free_semicolon(&mut self, token: &Token<'a>) {
        self.spaces.push_str(token.content);

        if !self.tree.is_container(self.current) {
            return;
        }
        let Some(prev) = self.tree.last(self.current) else {
            return;
        };
        let has_own_semicolon = self
            .tree
            .raws(prev)
            .own_semicolon
            .as_ref()
            .is_some_and(|semicolon| !semicolon.is_empty());
        if self.tree.type_name(prev) == "rule" && !has_own_semicolon {
            let spaces = std::mem::take(&mut self.spaces);
            let length = spaces.len();
            self.tree.raws_mut(prev).own_semicolon = Some(spaces);
            let mut end = self.get_position(token.start_or_zero());
            end.offset += length;
            self.set_end(prev, end);
        }
    }

    // --- Helpers ---------------------------------------------------------

    fn get_position(&self, offset: usize) -> Position {
        let (line, column) = self.input.from_offset(offset);
        Position {
            line,
            column,
            offset,
        }
    }

    fn init(&mut self, node: NodeId, offset: usize) {
        self.tree.push_child(self.current, node);
        self.tree.set_source(
            node,
            Source {
                input: Arc::clone(self.input),
                start: Some(self.get_position(offset)),
                end: None,
            },
        );
        self.tree.raws_mut(node).before = Some(std::mem::take(&mut self.spaces));
        if !matches!(self.tree.kind(node), NodeKind::Comment { .. }) {
            self.semicolon = false;
        }
    }

    fn set_start(&mut self, node: NodeId, position: Position) {
        if let Some(source) = self.tree.source_mut(node) {
            source.start = Some(position);
        }
    }

    fn set_end(&mut self, node: NodeId, position: Position) {
        if let Some(source) = self.tree.source_mut(node) {
            source.end = Some(position);
        }
    }

    /// Fills a value-like property, keeping the source text when cleaning it
    /// changed anything.
    ///
    /// Port of `Parser#raw()`.
    fn raw(&mut self, node: NodeId, prop: &str, tokens: &[Token<'a>], custom_property: bool) {
        let length = tokens.len();
        let mut value = String::new();
        let mut clean = true;

        for (i, token) in tokens.iter().enumerate() {
            if token.kind == TokenKind::Space && i == length - 1 && !custom_property {
                clean = false;
            } else if token.kind == TokenKind::Comment {
                let prev = if i == 0 { None } else { Some(tokens[i - 1].kind) };
                let next = tokens.get(i + 1).map(|token| token.kind);
                if !is_safe_comment_neighbor(prev) && !is_safe_comment_neighbor(next) {
                    if value.ends_with(',') {
                        clean = false;
                    } else {
                        value.push_str(token.content);
                    }
                } else {
                    clean = false;
                }
            } else {
                value.push_str(token.content);
            }
        }

        if !clean {
            let raw: String = tokens.iter().map(|token| token.content).collect();
            self.tree.set_raw_value(
                node,
                prop,
                RawValue {
                    raw,
                    value: value.clone(),
                },
            );
        }

        match prop {
            "value" => self.tree.set_value(node, value),
            "params" => self.tree.set_params(node, value),
            "selector" => self.tree.set_selector(node, value),
            _ => {}
        }
    }

    /// Index of the top-level `:` that separates a property from its value.
    ///
    /// Port of `Parser#colon()`.
    fn colon(&self, tokens: &[Token<'a>]) -> Result<Option<usize>, CssSyntaxError> {
        let mut brackets = 0i32;
        let mut prev: Option<Token<'a>> = None;

        for (i, token) in tokens.iter().enumerate() {
            let kind = token.kind;
            if kind == TokenKind::OpenParen {
                brackets += 1;
            }
            if kind == TokenKind::CloseParen {
                brackets -= 1;
            }
            if brackets == 0 && kind == TokenKind::Colon {
                match prev {
                    None => return Err(self.double_colon(token)),
                    // `progid:DXImageTransform...` is one IE value, not a
                    // property and a value.
                    Some(prev_token)
                        if prev_token.kind == TokenKind::Word
                            && prev_token.content == "progid" =>
                    {
                        continue;
                    }
                    Some(_) => return Ok(Some(i)),
                }
            }
            prev = Some(*token);
        }

        Ok(None)
    }

    fn check_missed_semicolon(&self, tokens: &[Token<'a>]) -> Result<(), CssSyntaxError> {
        let Some(colon) = self.colon(tokens)? else {
            return Ok(());
        };

        let mut found = 0;
        let mut token = tokens[colon - 1];
        for j in (0..colon).rev() {
            token = tokens[j];
            if token.kind != TokenKind::Space {
                found += 1;
                if found == 2 {
                    break;
                }
            }
        }

        // For a word such as `red` the caret belongs after the word, so the
        // reported position is the colon that follows it.
        let offset = if token.kind == TokenKind::Word {
            token.end.unwrap_or_default() + 1
        } else {
            token.start_or_zero()
        };
        Err(self.input.error_at_offset("Missed semicolon", offset))
    }

    // --- Errors ----------------------------------------------------------

    fn double_colon(&self, token: &Token<'a>) -> CssSyntaxError {
        let start = token.start_or_zero();
        self.input.error_range(
            "Double colon",
            Loc::Offset(start),
            Loc::Offset(start + token.content.len()),
            None,
        )
    }

    fn unclosed_block(&self) -> CssSyntaxError {
        let position = self
            .tree
            .source(self.current)
            .and_then(|source| source.start)
            .unwrap_or(Position {
                line: 1,
                column: 1,
                offset: 0,
            });
        self.input
            .error_at_line_col("Unclosed block", position.line, position.column)
    }

    fn unclosed_bracket(&self, bracket: &Token<'a>) -> CssSyntaxError {
        let start = bracket.start_or_zero();
        self.input.error_range(
            "Unclosed bracket",
            Loc::Offset(start),
            Loc::Offset(start + 1),
            None,
        )
    }

    fn unexpected_close(&self, token: &Token<'a>) -> CssSyntaxError {
        let start = token.start_or_zero();
        self.input.error_range(
            "Unexpected }",
            Loc::Offset(start),
            Loc::Offset(start + 1),
            None,
        )
    }

    fn unknown_word(&self, tokens: &[Token<'a>]) -> CssSyntaxError {
        let token = tokens[0];
        let start = token.start_or_zero();
        self.input.error_range(
            format!("Unknown word {}", token.content),
            Loc::Offset(start),
            Loc::Offset(start + token.content.len()),
            None,
        )
    }

    fn unnamed_atrule(&self, token: &Token<'a>) -> CssSyntaxError {
        let start = token.start_or_zero();
        self.input.error_range(
            "At-rule without name",
            Loc::Offset(start),
            Loc::Offset(start + token.content.len()),
            None,
        )
    }
}

/// `/\w/`
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Pops trailing spaces and comments, returning their text.
fn spaces_and_comments_from_end(tokens: &mut Vec<Token<'_>>) -> String {
    let mut spaces = String::new();
    while let Some(last) = tokens.last() {
        if last.kind != TokenKind::Space && last.kind != TokenKind::Comment {
            break;
        }
        let token = tokens.pop().expect("checked above");
        spaces.insert_str(0, token.content);
    }
    spaces
}

/// Removes leading spaces and comments, returning their text.
fn spaces_and_comments_from_start(tokens: &mut Vec<Token<'_>>) -> String {
    let mut spaces = String::new();
    while let Some(first) = tokens.first() {
        if first.kind != TokenKind::Space && first.kind != TokenKind::Comment {
            break;
        }
        spaces.push_str(first.content);
        tokens.remove(0);
    }
    spaces
}

/// Pops trailing spaces only.
fn spaces_from_end(tokens: &mut Vec<Token<'_>>) -> String {
    let mut spaces = String::new();
    while let Some(last) = tokens.last() {
        if last.kind != TokenKind::Space {
            break;
        }
        let token = tokens.pop().expect("checked above");
        spaces.insert_str(0, token.content);
    }
    spaces
}

/// Concatenates `tokens[from..]` and removes them.
fn string_from(tokens: &mut Vec<Token<'_>>, from: usize) -> String {
    let mut result = String::new();
    for token in &tokens[from..] {
        result.push_str(token.content);
    }
    tokens.truncate(from);
    result
}
