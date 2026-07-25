//! Source map generation and chaining.
//!
//! Expected CSS and map JSON in this file were produced by PostCSS 8.5.23 and
//! pasted in, so the assertions describe the reference implementation's output
//! rather than this crate's. `tools/diff-maps.mjs` re-checks the whole option
//! matrix against a live PostCSS.

use postcss::{Annotation, MapOptions, MapSetting, PrevMap, ProcessOptions, Processor};
use serde_json::{json, Value};

fn process(css: &str, opts: ProcessOptions) -> (String, Option<Value>) {
    let result = Processor::new().process(css, opts).expect("processes");
    let map = result
        .map
        .as_ref()
        .map(|map| serde_json::to_value(map.to_raw()).unwrap());
    (result.css, map)
}

fn external(from: &str, to: &str) -> ProcessOptions {
    ProcessOptions {
        from: Some(from.into()),
        to: Some(to.into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn generates_an_external_map() {
    let (css, map) = process("a { color: black }\n", external("a.css", "b.css"));

    assert_eq!(
        css,
        "a { color: black }\n\n/*# sourceMappingURL=b.css.map */"
    );
    assert_eq!(
        map.unwrap(),
        json!({
            "version": 3,
            "sources": ["a.css"],
            "names": [],
            "mappings": "AAAA,IAAI,aAAa",
            "file": "b.css",
            "sourcesContent": ["a { color: black }\n"],
        })
    );
}

#[test]
fn maps_nested_rules() {
    let source = "@media screen {\n  a {\n    color: black;\n  }\n}\n";
    let (css, map) = process(source, external("a.css", "b.css"));

    assert_eq!(
        css,
        "@media screen {\n  a {\n    color: black;\n  }\n}\n\n/*# sourceMappingURL=b.css.map */"
    );
    assert_eq!(
        map.unwrap()["mappings"],
        json!("AAAA;EACE;IACE,YAAY;EACd;AACF")
    );
}

#[test]
fn omits_sources_content_on_request() {
    let mut opts = external("a.css", "b.css");
    opts.map = Some(MapSetting::Options(MapOptions {
        inline: Some(false),
        sources_content: Some(false),
        ..Default::default()
    }));

    let (css, map) = process("a{color:black}", opts);
    assert_eq!(css, "a{color:black}\n/*# sourceMappingURL=b.css.map */");
    assert_eq!(
        map.unwrap(),
        json!({
            "version": 3,
            "sources": ["a.css"],
            "names": [],
            "mappings": "AAAA,EAAE,WAAW",
            "file": "b.css",
        })
    );
}

#[test]
fn resolves_paths_against_the_annotation_directory() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("out/b.css".into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            annotation: Some(Annotation::Path("maps/b.map".into())),
            ..Default::default()
        })),
        ..Default::default()
    };

    let (css, map) = process("a{color:black}", opts);
    assert_eq!(css, "a{color:black}\n/*# sourceMappingURL=maps/b.map */");

    let map = map.unwrap();
    assert_eq!(map["sources"], json!(["../../a.css"]));
    assert_eq!(map["file"], json!("../b.css"));
}

#[test]
fn writes_an_inline_map_by_default() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Enabled),
        ..Default::default()
    };

    let (css, map) = process("a{color:black}", opts);
    assert!(map.is_none(), "an inline map is not returned separately");
    assert!(css.contains("/*# sourceMappingURL=data:application/json;base64,"));

    // The embedded map decodes back to the same JSON.
    let base64 = css
        .rsplit("base64,")
        .next()
        .unwrap()
        .trim_end_matches(" */");
    let decoded = decode_base64(base64);
    let parsed: Value = serde_json::from_str(&decoded).expect("valid map JSON");
    assert_eq!(parsed["sources"], json!(["a.css"]));
    assert_eq!(parsed["mappings"], json!("AAAA,EAAE,WAAW"));
}

#[test]
fn skips_the_annotation_on_request() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            annotation: Some(Annotation::Disabled),
            ..Default::default()
        })),
        ..Default::default()
    };

    let (css, map) = process("a{color:black}", opts);
    assert_eq!(css, "a{color:black}");
    assert!(map.is_some());
}

#[test]
fn writes_no_map_when_disabled() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Disabled),
        ..Default::default()
    };

    let (css, map) = process("a{color:black}", opts);
    assert_eq!(css, "a{color:black}");
    assert!(map.is_none());
}

#[test]
fn writes_no_map_when_not_requested() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        ..Default::default()
    };

    let (css, map) = process("a{color:black}", opts);
    assert_eq!(css, "a{color:black}");
    assert!(map.is_none());
}

#[test]
fn removes_an_existing_annotation_comment() {
    let mut opts = external("a.css", "b.css");
    opts.map = Some(MapSetting::Options(MapOptions {
        inline: Some(false),
        sources_content: Some(false),
        ..Default::default()
    }));

    let (css, map) = process("a{}\n/*# sourceMappingURL=old.map */", opts);
    assert_eq!(css, "a{}\n/*# sourceMappingURL=b.css.map */");
    assert_eq!(map.unwrap()["mappings"], json!("AAAA,EAAE"));
}

#[test]
fn chains_onto_a_previous_map() {
    // A map standing in for a Sass compilation: b.css line 2 came from a.scss.
    let prev = json!({
        "version": 3,
        "file": "a.css",
        "sources": ["a.scss"],
        "sourcesContent": ["a\n  color: black\n"],
        "names": [],
        "mappings": "AAAA;EACE",
    })
    .to_string();

    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            prev: Some(PrevMap::Text(prev)),
            ..Default::default()
        })),
        ..Default::default()
    };

    let (_, map) = process("a {\n  color: black;\n}\n", opts);
    let map = map.unwrap();

    // Positions the previous map covers are retargeted to the Sass file;
    // positions it does not cover keep pointing at the intermediate CSS, so
    // both files appear in `sources`.
    assert_eq!(map["sources"], json!(["a.scss", "a.css"]));
    assert_eq!(map["mappings"], json!("AAAA;EACE,YAAA;ACCF"));
    assert_eq!(
        map["sourcesContent"],
        json!(["a\n  color: black\n", "a {\n  color: black;\n}\n"]),
        "the previous map's embedded source is carried over"
    );
}

#[test]
fn reads_a_previous_map_from_disk() {
    let dir = std::env::temp_dir().join(format!("postcss-rs-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let css_path = dir.join("a.css");
    let map_path = dir.join("a.css.map");

    let prev = json!({
        "version": 3,
        "file": "a.css",
        "sources": ["a.scss"],
        "sourcesContent": ["a\n  color: black\n"],
        "names": [],
        "mappings": "AAAA;EACE",
    })
    .to_string();
    std::fs::write(&map_path, &prev).unwrap();

    let css = "a {\n  color: black;\n}\n/*# sourceMappingURL=a.css.map */";
    std::fs::write(&css_path, css).unwrap();

    let opts = ProcessOptions {
        from: Some(css_path.to_string_lossy().into_owned()),
        to: Some(dir.join("b.css").to_string_lossy().into_owned()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            ..Default::default()
        })),
        ..Default::default()
    };

    let (out, map) = process(css, opts);
    // The stale annotation is dropped and a fresh one written.
    assert!(out.contains("/*# sourceMappingURL=b.css.map */"));
    assert!(!out.contains("a.css.map"));
    let map = map.unwrap();
    assert_eq!(map["sources"][0], json!("a.scss"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn refuses_map_files_outside_the_css_directory() {
    // A `sourceMappingURL` escaping the CSS file's directory is not read
    // unless `unsafe_map` is set.
    let dir = std::env::temp_dir().join(format!("postcss-rs-unsafe-{}", std::process::id()));
    let nested = dir.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        dir.join("outside.map"),
        "{\"version\":3,\"sources\":[],\"names\":[],\"mappings\":\"\"}",
    )
    .unwrap();

    let css = "a{}\n/*# sourceMappingURL=../outside.map */";
    let css_path = nested.join("a.css");
    std::fs::write(&css_path, css).unwrap();

    let input = postcss::Input::new(
        css,
        postcss::InputOptions {
            from: Some(css_path.to_string_lossy().into_owned()),
            ..Default::default()
        },
    );
    assert!(input.map.is_none(), "escaping map must not be loaded");

    let input = postcss::Input::new(
        css,
        postcss::InputOptions {
            from: Some(css_path.to_string_lossy().into_owned()),
            unsafe_map: true,
            ..Default::default()
        },
    );
    assert!(
        input.map.is_some(),
        "unsafe_map opts in to reading the escaping map"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn overrides_sources_with_map_from() {
    let opts = ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            from: Some("webpack://./a.css".into()),
            ..Default::default()
        })),
        ..Default::default()
    };

    let (_, map) = process("a{color:black}", opts);
    assert_eq!(map.unwrap()["sources"], json!(["webpack://./a.css"]));
}

#[test]
fn uses_crlf_before_the_annotation_when_the_css_does() {
    let (css, _) = process("a{\r\n  color: black\r\n}", external("a.css", "b.css"));
    assert!(css.ends_with("\r\n/*# sourceMappingURL=b.css.map */"));
}

fn decode_base64(text: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let Some(value) = CHARS.iter().position(|&c| c == byte) else {
            continue;
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    String::from_utf8(out).expect("valid UTF-8")
}
