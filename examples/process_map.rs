//! Processes CSS and prints the result plus its source map as JSON.
//!
//! Used by `tools/diff-maps.mjs` to diff map generation against the JS
//! implementation.
//!
//! ```sh
//! cargo run --example process_map -- --from a.css --to b.css --inline false
//! ```

use std::io::Read;

use postcss::{Annotation, MapOptions, MapSetting, PrevMap, ProcessOptions, Processor};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };

    let mut map = MapOptions::default();
    if let Some(inline) = flag("--inline") {
        map.inline = Some(inline == "true");
    }
    if let Some(sources_content) = flag("--sources-content") {
        map.sources_content = Some(sources_content == "true");
    }
    if let Some(annotation) = flag("--annotation") {
        map.annotation = Some(match annotation.as_str() {
            "true" => Annotation::Enabled,
            "false" => Annotation::Disabled,
            path => Annotation::Path(path.to_string()),
        });
    }
    if let Some(from) = flag("--map-from") {
        map.from = Some(from);
    }
    if flag("--absolute").as_deref() == Some("true") {
        map.absolute = true;
    }
    if let Some(prev) = flag("--prev") {
        map.prev = Some(PrevMap::Text(prev));
    }

    let opts = ProcessOptions {
        from: flag("--from"),
        to: flag("--to"),
        map: match flag("--map").as_deref() {
            Some("false") => Some(MapSetting::Disabled),
            Some("true") => Some(MapSetting::Enabled),
            _ => Some(MapSetting::Options(map)),
        },
        ..Default::default()
    };

    let mut css = String::new();
    std::io::stdin()
        .read_to_string(&mut css)
        .expect("readable stdin");

    match Processor::new().process(css, opts) {
        Ok(result) => {
            let map = result
                .map
                .as_ref()
                .map(|map| serde_json::to_value(map.to_raw()).expect("serializable"));
            println!(
                "{}",
                json!({ "css": result.css, "map": map })
            );
        }
        Err(error) => println!("{}", json!({ "error": error.reason })),
    }
}
