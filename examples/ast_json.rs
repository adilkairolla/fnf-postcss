//! Dumps the AST and the stringified CSS as JSON, in the same shape the
//! `postcss-parser-tests` `jsonify()` helper produces.
//!
//! Used by `tools/diff-postcss.mjs` to diff this crate against the JS
//! implementation.
//!
//! ```sh
//! cargo run --example ast_json -- file.css
//! cat file.css | cargo run --example ast_json
//! ```

use std::io::Read;

use serde_json::{json, Value};

fn clean(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        object.remove("inputs");
        if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) {
            source.remove("input");
            source.remove("inputId");
        }
        if let Some(nodes) = object.get_mut("nodes").and_then(Value::as_array_mut) {
            for node in nodes {
                clean(node);
            }
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1);
    let css = match &arg {
        Some(path) => std::fs::read_to_string(path).expect("readable file"),
        None => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .expect("readable stdin");
            buffer
        }
    };

    let opts = postcss::InputOptions {
        from: arg.clone(),
        // Never read a neighbouring .map file: the diff compares parsing only.
        map: Some(postcss::MapSetting::Disabled),
        ..Default::default()
    };

    let output = match postcss::parse_with_options(css, opts) {
        Ok(tree) => {
            let mut ast = postcss::json::to_json(&tree);
            clean(&mut ast);
            json!({ "ast": ast, "css": tree.to_css() })
        }
        Err(error) => json!({
            "error": {
                "reason": error.reason,
                "line": error.line,
                "column": error.column,
                "endLine": error.end_line,
                "endColumn": error.end_column,
            }
        }),
    };

    println!("{}", serde_json::to_string(&output).expect("serializable"));
}
