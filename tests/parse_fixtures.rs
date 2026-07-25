//! Conformance suite: parses every case from `postcss-parser-tests` and
//! compares the AST against the JSON PostCSS produces, then checks that
//! stringifying it reproduces the input byte for byte.
//!
//! Fixtures in `tests/fixtures/cases` are copied verbatim from
//! `postcss-parser-tests` (MIT, see `tests/fixtures/LICENSE-postcss-parser-tests`).

use std::fs;
use std::path::{Path, PathBuf};

use postcss::{json, parse_with_options, InputOptions};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Strips the fields `jsonify()` drops before comparing.
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

/// Every case as `(name, css, expected_json)`.
fn each_test() -> Vec<(String, String, Value)> {
    let dir = fixtures_dir();
    let extra: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("extra-cases.json")).unwrap()).unwrap();

    let mut cases: Vec<(String, String, Value)> = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir.join("cases"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();

    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let json: Value = serde_json::from_str(fs::read_to_string(&path).unwrap().trim()).unwrap();

        let css = match extra.get(&name).and_then(Value::as_str) {
            Some(css) => css.to_string(),
            None => fs::read_to_string(path.with_extension("css"))
                .unwrap()
                .trim()
                .to_string(),
        };

        cases.push((format!("{}.css", name), css, json));
    }

    assert!(cases.len() >= 30, "fixtures are missing");
    cases
}

#[test]
fn parses_every_upstream_case() {
    let mut failures: Vec<String> = Vec::new();

    for (name, css, expected) in each_test() {
        let css = css.replace("\r\n", "\n");
        let tree = match parse_with_options(
            css.clone(),
            InputOptions {
                from: Some(name.clone()),
                ..Default::default()
            },
        ) {
            Ok(tree) => tree,
            Err(error) => {
                failures.push(format!("{}: failed to parse: {}", name, error.message));
                continue;
            }
        };

        let mut actual = json::to_json(&tree);
        clean(&mut actual);

        if actual != expected {
            failures.push(format!(
                "{}: AST mismatch\n  expected: {}\n  actual:   {}",
                name,
                serde_json::to_string(&expected).unwrap(),
                serde_json::to_string(&actual).unwrap(),
            ));
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
fn stringifies_every_upstream_case_back_to_the_input() {
    let mut failures: Vec<String> = Vec::new();

    for (name, css, _) in each_test() {
        // PostCSS strips a BOM from `input.css`, so it is not part of the
        // output either; `input.has_bom` records that it was there.
        let css = css.replace("\r\n", "\n");
        let css = css.strip_prefix('\u{feff}').unwrap_or(&css).to_string();
        let Ok(tree) = parse_with_options(
            css.clone(),
            InputOptions {
                from: Some(name.clone()),
                ..Default::default()
            },
        ) else {
            failures.push(format!("{}: failed to parse", name));
            continue;
        };

        let output = tree.to_css();
        if output != css {
            failures.push(format!(
                "{}: round-trip mismatch\n  input:  {:?}\n  output: {:?}",
                name, css, output
            ));
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
fn round_trips_through_json() {
    for (name, css, _) in each_test() {
        let css = css.replace("\r\n", "\n");
        let css = css.strip_prefix('\u{feff}').unwrap_or(&css).to_string();
        let tree = parse_with_options(
            css.clone(),
            InputOptions {
                from: Some(name.clone()),
                ..Default::default()
            },
        )
        .unwrap();

        let value = json::to_json(&tree);
        let rebuilt = json::from_json(&value).unwrap_or_else(|e| panic!("{}: {}", name, e));
        assert_eq!(rebuilt.to_css(), css, "{}: JSON round-trip changed the CSS", name);
        assert_eq!(
            json::to_json(&rebuilt),
            value,
            "{}: JSON round-trip changed the AST",
            name
        );
    }
}
