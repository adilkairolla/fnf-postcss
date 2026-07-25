//! Port of the fnf-web `viewport-units-downlevel` PostCSS plugin, used to check
//! this crate against the JS one on the app's real CSS.
//!
//! The JS original lives at
//! `packages/config/src/postcss/viewport-units-downlevel.cjs`. It leans on
//! `OnceExit`, `walkRules`, `walkDecls` with a parent-identity filter, `clone`,
//! `removeAll`, `append`, `new AtRule({ source })` and `after` — i.e. most of
//! the container API a real plugin touches.
//!
//! Reads CSS on stdin, writes the transformed CSS to stdout.
//!
//! ```sh
//! cargo run --release --example fnf_viewport_downlevel < in.css
//! ```

use std::io::Read;

use postcss::{
    CssSyntaxError, NewNode, NodeId, NodeKind, Plugin, PluginContext, ProcessOptions, Processor,
    Tree,
};

const SUPPORTS: &str = "not (height: 1dvh)";

/// The `UNIT_MAP` of the JS plugin, in the same order.
const UNIT_MAP: &[(&str, &str)] = &[
    ("dvh", "vh"),
    ("svh", "vh"),
    ("lvh", "vh"),
    ("dvw", "vw"),
    ("svw", "vw"),
    ("lvw", "vw"),
    ("dvb", "vh"),
    ("svb", "vh"),
    ("lvb", "vh"),
    ("dvi", "vw"),
    ("svi", "vw"),
    ("lvi", "vw"),
    ("dvmin", "vmin"),
    ("svmin", "vmin"),
    ("lvmin", "vmin"),
    ("dvmax", "vmax"),
    ("svmax", "vmax"),
    ("lvmax", "vmax"),
];

fn mapped_unit(unit: &str) -> Option<&'static str> {
    let lower = unit.to_lowercase();
    UNIT_MAP
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| *to)
}

/// `DETECT`: a digit immediately followed by one of the units, then a word
/// boundary. Stands in for the JS regex.
fn detect(value: &str) -> bool {
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if !byte.is_ascii_digit() {
            continue;
        }
        let rest = &value[index + 1..];
        for (unit, _) in UNIT_MAP {
            if rest.len() >= unit.len() && rest[..unit.len()].eq_ignore_ascii_case(unit) {
                let after = rest.as_bytes().get(unit.len());
                let boundary = match after {
                    None => true,
                    Some(next) => !(next.is_ascii_alphanumeric() || *next == b'_'),
                };
                if boundary {
                    return true;
                }
            }
        }
    }
    false
}

/// `postcss-value-parser`'s `unit()`, narrowed to what this plugin needs: split
/// a word into its leading number and trailing unit.
fn split_unit(word: &str) -> Option<(&str, &str)> {
    let bytes = word.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index = 1;
    }
    let digits_start = index;
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
        index += 1;
    }
    if matches!(bytes.get(index), Some(b'.')) {
        index += 1;
        while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
            index += 1;
        }
    }
    // No digits at all is not a number.
    if word[digits_start..index]
        .bytes()
        .all(|byte| !byte.is_ascii_digit())
    {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let mut exponent = index + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let start = exponent;
        while matches!(bytes.get(exponent), Some(byte) if byte.is_ascii_digit()) {
            exponent += 1;
        }
        if exponent > start {
            index = exponent;
        }
    }
    Some((&word[..index], &word[index..]))
}

/// `downlevel(value)`: rewrite every word whose unit is in the map. Word
/// boundaries follow `postcss-value-parser`'s separators, so the rest of the
/// value — spacing, commas, nesting — comes back unchanged.
fn downlevel(value: &str) -> Option<String> {
    if !detect(value) {
        return None;
    }

    let is_separator = |byte: u8| {
        matches!(
            byte,
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b',' | b'/' | b'(' | b')' | b'\'' | b'"'
        )
    };

    let mut out = String::with_capacity(value.len());
    let mut changed = false;
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if is_separator(bytes[index]) {
            out.push(bytes[index] as char);
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && !is_separator(bytes[index]) {
            index += 1;
        }
        let word = &value[start..index];
        match split_unit(word).and_then(|(number, unit)| {
            mapped_unit(unit).map(|replacement| format!("{number}{replacement}"))
        }) {
            Some(rewritten) => {
                out.push_str(&rewritten);
                changed = true;
            }
            None => out.push_str(word),
        }
    }

    if changed {
        Some(out)
    } else {
        None
    }
}

/// One declaration to re-emit inside the `@supports` fallback.
struct Fallback {
    important: bool,
    prop: String,
    value: String,
}

struct ViewportUnitsDownlevel;

impl Plugin for ViewportUnitsDownlevel {
    fn name(&self) -> &str {
        "viewport-units-downlevel"
    }

    fn once_exit(&self, tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        // `Map<rule, decls>` in the JS plugin: insertion-ordered, one entry per
        // rule that needs a fallback.
        let mut fallbacks: Vec<(NodeId, Vec<Fallback>)> = Vec::new();

        let root = tree.root();
        tree.walk_ref(root, |tree, rule| {
            if !matches!(tree.kind(rule), NodeKind::Rule { .. }) {
                return;
            }
            let mut decls: Vec<Fallback> = Vec::new();
            tree.walk_ref(rule, |tree, decl| {
                if !matches!(tree.kind(decl), NodeKind::Decl { .. }) {
                    return;
                }
                // Only direct children, as in the JS plugin.
                if tree.parent(decl) != Some(rule) {
                    return;
                }
                let Some(prop) = tree.prop(decl) else { return };
                if prop.starts_with("--") {
                    return;
                }
                let Some(value) = tree.value(decl) else {
                    return;
                };
                if let Some(rewritten) = downlevel(value) {
                    decls.push(Fallback {
                        important: tree.important(decl),
                        prop: prop.to_string(),
                        value: rewritten,
                    });
                }
            });
            if !decls.is_empty() {
                fallbacks.push((rule, decls));
            }
        });

        for (rule, decls) in fallbacks {
            let fallback = tree.clone_node(rule);
            tree.remove_all(fallback);
            for decl in decls {
                let new = NewNode::decl(decl.prop, decl.value).important(decl.important);
                tree.append(fallback, new)?;
            }

            let mut supports = NewNode::at_rule("supports", SUPPORTS);
            supports.source = tree.source(rule).cloned();
            let supports = tree.create(supports);
            tree.append(supports, fallback)?;
            tree.insert_after(rule, supports)?;
        }
        Ok(())
    }
}

fn main() {
    let mut css = String::new();
    std::io::stdin()
        .read_to_string(&mut css)
        .expect("readable stdin");

    let from = std::env::args().nth(1);
    let result = Processor::new()
        .with(ViewportUnitsDownlevel)
        .process(
            css,
            ProcessOptions {
                from,
                map: Some(postcss::MapSetting::Disabled),
                ..Default::default()
            },
        )
        .expect("processes");

    print!("{}", result.css);
}
