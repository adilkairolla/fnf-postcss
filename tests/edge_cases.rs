//! Regression suite over an adversarial corpus.
//!
//! `tests/fixtures/edge-expected.json` records what PostCSS 8.5.23 produces for
//! every file in `tests/fixtures/edge` — AST, output CSS, or error position —
//! translated into this crate's position model by
//! `tools/snapshot-postcss.mjs`. The expectations therefore come from the
//! reference implementation, not from this crate.

use std::fs;
use std::path::{Path, PathBuf};

use postcss::{json, parse_with_options, InputOptions, MapSetting};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

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

fn expectations() -> serde_json::Map<String, Value> {
    let path = fixtures_dir().join("edge-expected.json");
    let text = fs::read_to_string(path).expect("snapshot exists");
    serde_json::from_str::<Value>(&text)
        .expect("snapshot is valid JSON")
        .as_object()
        .expect("snapshot is an object")
        .clone()
}

#[test]
fn matches_postcss_on_every_edge_case() {
    let expected = expectations();
    assert!(expected.len() >= 40, "snapshot looks truncated");
    let mut failures: Vec<String> = Vec::new();

    for (name, expected) in &expected {
        let css = fs::read_to_string(fixtures_dir().join("edge").join(name))
            .unwrap_or_else(|_| panic!("missing fixture {}", name));

        let opts = InputOptions {
            from: Some(name.clone()),
            map: Some(MapSetting::Disabled),
            ..Default::default()
        };

        match parse_with_options(css, opts) {
            Ok(tree) => {
                if let Some(error) = expected.get("error") {
                    failures.push(format!(
                        "{}: parsed successfully, but PostCSS reports {}",
                        name, error
                    ));
                    continue;
                }

                let mut actual = json::to_json(&tree);
                clean(&mut actual);
                if Some(&actual) != expected.get("ast") {
                    failures.push(format!(
                        "{}: AST mismatch\n  expected: {}\n  actual:   {}",
                        name,
                        expected
                            .get("ast")
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        actual
                    ));
                }

                let output = Value::String(tree.to_css());
                if Some(&output) != expected.get("css") {
                    failures.push(format!(
                        "{}: output mismatch\n  expected: {}\n  actual:   {}",
                        name,
                        expected
                            .get("css")
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        output
                    ));
                }
            }
            Err(error) => {
                let Some(expected_error) = expected.get("error") else {
                    failures.push(format!(
                        "{}: failed to parse with {:?}, but PostCSS succeeds",
                        name, error.reason
                    ));
                    continue;
                };

                let actual = serde_json::json!({
                    "reason": error.reason,
                    "line": error.line,
                    "column": error.column,
                    "endLine": error.end_line,
                    "endColumn": error.end_column,
                });
                if &actual != expected_error {
                    failures.push(format!(
                        "{}: error mismatch\n  expected: {}\n  actual:   {}",
                        name, expected_error, actual
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} case(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn round_trips_every_parseable_edge_case() {
    for (name, expected) in &expectations() {
        if expected.get("error").is_some() {
            continue;
        }
        let css = fs::read_to_string(fixtures_dir().join("edge").join(name)).unwrap();
        let tree = parse_with_options(
            css.clone(),
            InputOptions {
                from: Some(name.clone()),
                map: Some(MapSetting::Disabled),
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("{}: {}", name, error.reason));

        assert_eq!(tree.to_css(), css, "{} did not round-trip", name);
    }
}
