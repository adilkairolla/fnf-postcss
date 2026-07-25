//! Times parse and stringify over a CSS file.
//!
//! ```sh
//! cargo run --release --example bench -- path/to.css [iterations]
//! ```
//!
//! `tools/bench-postcss.mjs` runs the same loop through the JS implementation
//! for comparison.

use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: bench <file.css> [iterations]");
    let iterations: usize = args
        .next()
        .map(|value| value.parse().expect("iterations must be a number"))
        .unwrap_or(20);

    let css = std::fs::read_to_string(&path).expect("readable file");
    let opts = postcss::InputOptions {
        from: Some(path.clone()),
        // Never touch the filesystem looking for a neighbouring map.
        map: Some(postcss::MapSetting::Disabled),
        ..Default::default()
    };

    // Warm up, and fail loudly before timing if the file does not parse.
    let tree = postcss::parse_with_options(css.clone(), opts.clone()).expect("parses");
    assert_eq!(tree.to_css(), css, "round-trip is not byte-exact");

    let mut parse_total = 0f64;
    let mut stringify_total = 0f64;

    for _ in 0..iterations {
        let start = Instant::now();
        let tree = postcss::parse_with_options(css.clone(), opts.clone()).expect("parses");
        parse_total += start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        let out = tree.to_css();
        stringify_total += start.elapsed().as_secs_f64() * 1000.0;
        // Keep the optimizer from discarding the work.
        assert!(!out.is_empty());
    }

    let parse = parse_total / iterations as f64;
    let stringify = stringify_total / iterations as f64;
    println!(
        "{}: {} KiB, {} iterations",
        path,
        css.len() / 1024,
        iterations
    );
    println!("  parse:     {:>8.2} ms", parse);
    println!("  stringify: {:>8.2} ms", stringify);
    println!("  total:     {:>8.2} ms", parse + stringify);
}
