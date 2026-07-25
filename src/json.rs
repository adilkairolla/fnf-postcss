//! Converting a [`Tree`] to and from JSON.
//!
//! Ports `Node#toJSON()` and `lib/fromJSON.js`. The shape matches the JS
//! implementation, so trees can be handed to and from JS tooling.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::input::Input;
use crate::node::{NewNode, NodeId, NodeKind, Position, RawValue, Raws, Source};
use crate::tree::Tree;

/// Serializes a whole tree, including an `inputs` array.
pub fn to_json(tree: &Tree) -> Value {
    node_to_json(tree, tree.root())
}

/// Serializes one node and its subtree.
pub fn node_to_json(tree: &Tree, id: NodeId) -> Value {
    let inputs = tree.inputs();
    let mut value = node_to_json_inner(tree, id, &inputs);

    if let Some(object) = value.as_object_mut() {
        object.insert(
            "inputs".to_string(),
            Value::Array(inputs.iter().map(|input| input_to_json(input)).collect()),
        );
    }
    value
}

fn node_to_json_inner(tree: &Tree, id: NodeId, inputs: &[Arc<Input>]) -> Value {
    let node = tree.node(id);
    let mut object = Map::new();

    object.insert("raws".to_string(), raws_to_json(&node.raws, &node.kind));
    object.insert("type".to_string(), json!(node.type_name()));

    match &node.kind {
        NodeKind::AtRule { name, params } => {
            object.insert("name".to_string(), json!(name));
            object.insert("params".to_string(), json!(params));
        }
        NodeKind::Comment { text } => {
            object.insert("text".to_string(), json!(text));
        }
        NodeKind::Decl {
            prop,
            value,
            important,
        } => {
            object.insert("prop".to_string(), json!(prop));
            if *important {
                object.insert("important".to_string(), json!(true));
            }
            object.insert("value".to_string(), json!(value));
        }
        NodeKind::Rule { selector } => {
            object.insert("selector".to_string(), json!(selector));
        }
        NodeKind::Root | NodeKind::Document => {}
    }

    if let Some(source) = &node.source {
        let input_id = inputs
            .iter()
            .position(|input| Arc::ptr_eq(input, &source.input))
            .unwrap_or(0);
        let mut source_object = Map::new();
        if let Some(end) = source.end {
            source_object.insert("end".to_string(), position_to_json(end));
        }
        source_object.insert("inputId".to_string(), json!(input_id));
        if let Some(start) = source.start {
            source_object.insert("start".to_string(), position_to_json(start));
        }
        object.insert("source".to_string(), Value::Object(source_object));
    }

    if let Some(children) = tree.nodes(id) {
        let nodes = children
            .iter()
            .map(|&child| node_to_json_inner(tree, child, inputs))
            .collect();
        object.insert("nodes".to_string(), Value::Array(nodes));
    }

    Value::Object(object)
}

fn position_to_json(position: Position) -> Value {
    json!({
        "column": position.column,
        "line": position.line,
        "offset": position.offset,
    })
}

fn raws_to_json(raws: &Raws, kind: &NodeKind) -> Value {
    let mut object = Map::new();

    fn insert(object: &mut Map<String, Value>, key: &str, value: Option<&String>) {
        if let Some(value) = value {
            object.insert(key.to_string(), json!(value));
        }
    }

    insert(&mut object, "before", raws.before.as_ref());
    insert(&mut object, "between", raws.between.as_ref());
    insert(&mut object, "afterName", raws.after_name.as_ref());
    insert(&mut object, "left", raws.left.as_ref());
    insert(&mut object, "right", raws.right.as_ref());
    insert(&mut object, "important", raws.important.as_ref());
    insert(&mut object, "indent", raws.indent.as_ref());

    if let Some(semicolon) = raws.semicolon {
        object.insert("semicolon".to_string(), json!(semicolon));
    }
    insert(&mut object, "ownSemicolon", raws.own_semicolon.as_ref());
    insert(&mut object, "after", raws.after.as_ref());

    let raw_value = match kind {
        NodeKind::Decl { .. } => raws.value.as_ref().map(|value| ("value", value)),
        NodeKind::AtRule { .. } => raws.params.as_ref().map(|value| ("params", value)),
        NodeKind::Rule { .. } => raws.selector.as_ref().map(|value| ("selector", value)),
        _ => None,
    };
    if let Some((key, raw)) = raw_value {
        object.insert(
            key.to_string(),
            json!({ "raw": raw.raw, "value": raw.value }),
        );
    }

    for (key, value) in &raws.extra {
        object.insert(key.clone(), value.clone());
    }

    Value::Object(object)
}

fn input_to_json(input: &Input) -> Value {
    let mut object = Map::new();
    object.insert("hasBOM".to_string(), json!(input.has_bom));
    object.insert("css".to_string(), json!(input.css()));
    if let Some(file) = &input.file {
        object.insert("file".to_string(), json!(file));
    }
    if let Some(id) = &input.id {
        object.insert("id".to_string(), json!(id));
    }
    Value::Object(object)
}

/// Rebuilds a tree from [`to_json`] output.
///
/// Port of `lib/fromJSON.js`.
pub fn from_json(value: &Value) -> Result<Tree, String> {
    let inputs: Vec<Arc<Input>> = value
        .get("inputs")
        .and_then(Value::as_array)
        .map(|inputs| inputs.iter().map(input_from_json).collect())
        .unwrap_or_default();

    let mut tree = match value.get("type").and_then(Value::as_str) {
        Some("document") => Tree::new_document(),
        Some("root") => Tree::new(),
        Some(other) => return Err(format!("Cannot build a tree from a {} node", other)),
        None => return Err("Missing node type".to_string()),
    };

    let root = tree.root();
    apply_json(&mut tree, root, value, &inputs)?;
    Ok(tree)
}

/// Fills an existing node from JSON, then creates its children.
fn apply_json(
    tree: &mut Tree,
    id: NodeId,
    value: &Value,
    inputs: &[Arc<Input>],
) -> Result<(), String> {
    let object = value.as_object().ok_or("Node is not an object")?;

    if let Some(raws) = object.get("raws") {
        let kind = tree.kind(id).clone();
        *tree.raws_mut(id) = raws_from_json(raws, &kind);
    }
    if let Some(source) = object.get("source") {
        if let Some(source) = source_from_json(source, inputs) {
            tree.node_mut(id).source = Some(source);
        }
    }

    if let Some(children) = object.get("nodes").and_then(Value::as_array) {
        tree.make_container(id);
        for child in children {
            let new_node = new_node_from_json(child)?;
            let child_id = tree.create(new_node);
            tree.push_child(id, child_id);
            apply_json(tree, child_id, child, inputs)?;
        }
    }

    Ok(())
}

fn new_node_from_json(value: &Value) -> Result<NewNode, String> {
    let object = value.as_object().ok_or("Node is not an object")?;
    let type_name = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Missing node type")?;
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    let mut node = match type_name {
        "atrule" => NewNode::at_rule(text("name"), text("params")),
        "comment" => NewNode::comment(text("text")),
        "decl" => NewNode::decl(text("prop"), text("value")).important(
            object
                .get("important")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        "root" => NewNode::root(),
        "rule" => NewNode::rule(text("selector")),
        other => return Err(format!("Unknown node type {}", other)),
    };

    // The presence of `nodes` is what makes an at-rule a block.
    if object.contains_key("nodes") && node.nodes.is_none() {
        node.nodes = Some(Vec::new());
    }
    Ok(node)
}

fn raws_from_json(value: &Value, kind: &NodeKind) -> Raws {
    let mut raws = Raws::default();
    let Some(object) = value.as_object() else {
        return raws;
    };

    for (key, value) in object {
        match (key.as_str(), value) {
            ("semicolon", Value::Bool(semicolon)) => raws.semicolon = Some(*semicolon),
            ("value" | "params" | "selector", Value::Object(raw)) => {
                let raw_value = RawValue {
                    raw: raw
                        .get("raw")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    value: raw
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                };
                // Keep the key the node's type actually uses.
                let expected = match kind {
                    NodeKind::Decl { .. } => "value",
                    NodeKind::AtRule { .. } => "params",
                    NodeKind::Rule { .. } => "selector",
                    _ => "",
                };
                if key == expected {
                    raws.set_raw_value(key, raw_value);
                } else {
                    raws.extra.insert(key.clone(), value.clone());
                }
            }
            (key, Value::String(text)) => raws.set_str(key, text.clone()),
            (key, other) => {
                raws.extra.insert(key.to_string(), other.clone());
            }
        }
    }

    raws
}

fn source_from_json(value: &Value, inputs: &[Arc<Input>]) -> Option<Source> {
    let object = value.as_object()?;
    let input_id = object.get("inputId").and_then(Value::as_u64).unwrap_or(0) as usize;
    let input = inputs.get(input_id)?;

    Some(Source {
        input: Arc::clone(input),
        start: object.get("start").and_then(position_from_json),
        end: object.get("end").and_then(position_from_json),
    })
}

fn position_from_json(value: &Value) -> Option<Position> {
    let object = value.as_object()?;
    Some(Position {
        line: object.get("line").and_then(Value::as_u64)? as usize,
        column: object.get("column").and_then(Value::as_u64)? as usize,
        offset: object.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
    })
}

fn input_from_json(value: &Value) -> Arc<Input> {
    let css = value
        .get("css")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let has_bom = value
        .get("hasBOM")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let file = value
        .get("file")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = value.get("id").and_then(Value::as_str).map(str::to_string);

    Arc::new(Input::from_json_parts(css, has_bom, file, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_tree() {
        let css = "a /* c */ {\n  color: red !important;\n}\n@media screen { b { top: 0 } }\n";
        let tree = crate::parse(css).unwrap();
        let json = to_json(&tree);
        let rebuilt = from_json(&json).unwrap();
        assert_eq!(rebuilt.to_css(), css);
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn keeps_block_less_at_rules_block_less() {
        let tree = crate::parse("@charset \"utf-8\";").unwrap();
        let rebuilt = from_json(&to_json(&tree)).unwrap();
        assert_eq!(rebuilt.to_css(), "@charset \"utf-8\";");
        let at_rule = rebuilt.first(rebuilt.root()).unwrap();
        assert!(rebuilt.nodes(at_rule).is_none());
    }
}
