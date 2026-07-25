# postcss-rs

A Rust port of [PostCSS](https://github.com/postcss/postcss): parse CSS into an
AST, transform it with plugins, and write it back out — preserving the original
formatting byte for byte, with source map support.

Ported from PostCSS `main` (8.5.23). No runtime dependencies beyond `serde` and
`serde_json`; the `source-map-js`, `nanoid` and `picocolors` dependencies of the
JS version are reimplemented here.

```rust
use postcss::{parse, NewNode};

let mut tree = parse("a { color: red }")?;

tree.walk_decls(|tree, decl| {
    if tree.value(decl) == Some("red") {
        tree.set_value(decl, "green");
    }
});

assert_eq!(tree.to_css(), "a { color: green }");
```

## What is here

| Area | JS module | Rust module |
| --- | --- | --- |
| Tokenizer | `tokenize.js` | [`tokenize`](src/tokenize.rs) |
| Parser | `parser.js` | [`parser`](src/parser.rs) |
| AST nodes | `node.js`, `root.js`, `rule.js`, `at-rule.js`, `declaration.js`, `comment.js`, `document.js` | [`node`](src/node.rs) |
| Container API | `container.js` | [`tree`](src/tree.rs) |
| Stringifier | `stringifier.js`, `stringify.js` | [`stringifier`](src/stringifier.rs) |
| Input & positions | `input.js` | [`input`](src/input.rs) |
| Errors | `css-syntax-error.js`, `terminal-highlight.js` | [`error`](src/error.rs), [`terminal_highlight`](src/terminal_highlight.rs) |
| Value splitting | `list.js` | [`list`](src/list.rs) |
| Plugins & pipeline | `processor.js`, `lazy-result.js` | [`processor`](src/processor.rs) |
| Result & warnings | `result.js`, `warning.js` | [`result`](src/result.rs) |
| Source maps | `map-generator.js`, `previous-map.js` | [`map_generator`](src/map_generator.rs), [`previous_map`](src/previous_map.rs), [`source_map`](src/source_map.rs) |
| JSON | `fromJSON.js`, `Node#toJSON` | [`json`](src/json.rs) |

## The tree

PostCSS in JS hands you node objects with `parent` pointers you mutate freely.
Rust's ownership rules make that shape painful, so nodes live in an arena on
[`Tree`] and are addressed by [`NodeId`]:

```rust
use postcss::{parse, NewNode};

let mut tree = parse("@media print { a { color: red } }")?;
let media = tree.first(tree.root()).unwrap();
let rule = tree.first(media).unwrap();

// Ids stay valid across mutations, so you can hold one while editing siblings.
tree.append(rule, NewNode::decl("top", "0"))?;
tree.insert_before(rule, NewNode::comment("note"))?;

assert_eq!(
    tree.to_css(),
    "@media print { /* note */ a { color: red; top: 0 } }"
);
```

Everything `container.js` offers is there: `each`, `walk`, `walk_decls`,
`walk_rules`, `walk_at_rules`, `walk_comments`, `append`, `prepend`,
`insert_before`, `insert_after`, `remove`, `remove_all`, `replace_with`,
`clone_node`, `clone_before`, `clone_after`, `every`, `some`, `index`,
`replace_values`, `clean_raws`.

Mutating during a walk behaves as it does in JS: inserting or removing shifts the
live cursor, so every remaining node is still visited exactly once.

`append` and friends take anything that converts into [`Insertable`] — a CSS
string to parse, a [`NewNode`] to build, an existing [`NodeId`] to move, another
[`Tree`] to adopt, or a `Vec` of those.

## Plugins

A plugin implements [`Plugin`]. Hooks default to no-ops, so implement only what
you need:

```rust
use postcss::{CssSyntaxError, NodeId, Plugin, PluginContext, Processor, ProcessOptions, Tree};

struct Prefixer;

impl Plugin for Prefixer {
    fn name(&self) -> &str {
        "prefixer"
    }

    fn decl(&self, tree: &mut Tree, decl: NodeId, _ctx: &mut PluginContext)
        -> Result<(), CssSyntaxError>
    {
        if tree.prop(decl) == Some("user-select") {
            tree.set_prop(decl, "-webkit-user-select");
        }
        Ok(())
    }
}

let result = Processor::new()
    .with(Prefixer)
    .process("a { user-select: none }", ProcessOptions::default())?;

assert_eq!(result.css, "a { -webkit-user-select: none }");
```

`once` runs before any node visitor, `once_exit` after they settle, and
`root`/`rule`/`at_rule`/`decl`/`comment` visit nodes. A node a plugin changes is
marked dirty and visited again, so plugins see each other's output; a tree that
never settles is reported as an error rather than looping forever.

Warnings and messages go through [`PluginContext`]:

```rust,ignore
ctx.warn(tree, "Avoid !important", Some(decl), &NodeErrorOptions {
    word: Some("!important".into()),
    ..Default::default()
});
```

## Source maps

Maps matter for a real toolchain: Vite, webpack and Next all pass
`map: { prev, inline, annotation }` into PostCSS and expect `result.map` to chain
back to the original `.scss`/`.vue` file. That is implemented, including reading
an existing `sourceMappingURL` (inline or from disk) and retargeting the new map
through it.

```rust
use postcss::{MapOptions, MapSetting, ProcessOptions, Processor};

let result = Processor::new().process(
    "a { color: black }\n",
    ProcessOptions {
        from: Some("a.css".into()),
        to: Some("b.css".into()),
        map: Some(MapSetting::Options(MapOptions {
            inline: Some(false),
            ..Default::default()
        })),
        ..Default::default()
    },
)?;

assert_eq!(result.css, "a { color: black }\n\n/*# sourceMappingURL=b.css.map */");
assert!(result.map_json().unwrap().contains("\"mappings\":\"AAAA,IAAI,aAAa\""));
```

`inline`, `prev`, `sourcesContent`, `annotation`, `from` and `absolute` all
behave as documented for PostCSS, and the serialized map is byte-identical to the
JS output — including the base64 payload of an inline annotation.

By default a map file next to the CSS is only read when it sits inside the CSS
file's own directory; `unsafe_map` opts out of that check.

## Feature parity

The parser, stringifier, node/container API, error reporting, `list`, JSON
round-trip and source maps are complete: every upstream behaviour they have is
here, checked against the JS implementation (see [Verifying it](#verifying-it)).

The **plugin API is a subset**. Present: `Once`, `OnceExit`, `Root`, `Rule`,
`AtRule`, `Declaration`, `Comment`, the dirty-node re-visit loop, warnings and
messages. Missing:

| Upstream | Status |
| --- | --- |
| `RootExit`, `RuleExit`, `AtRuleExit`, `DeclarationExit`, `CommentExit` | not implemented |
| `Document` / `DocumentExit` listeners | not implemented (the node type exists) |
| `prepare(result)` for per-file plugin state | not implemented |
| Filtered listeners — `Declaration: { color(decl) {} }` | not implemented; match inside the hook |
| `Root#toResult`, `Processor#version`, `Result#processor`, `Result#lastPlugin` | not implemented |
| `Node#assign` | not applicable; use the setters |

Also absent, deliberately: async plugins and `LazyResult`, custom-syntax plugging
(`opts.parser`/`stringifier`/`syntax`), and `Stringifier` subclassing.

## Differences from the JS implementation

These are deliberate, and each one is exercised by the test suite:

- **Positions.** `offset` is a UTF-8 byte offset and `column` counts characters.
  JS counts UTF-16 code units for both. They agree for ASCII — which is all of
  CSS's own syntax — and differ only inside non-ASCII identifiers, strings and
  comments. `tools/diff-postcss.mjs` converts between the two models so the
  differential tests still compare positions exactly.
- **Nodes are ids, not references.** See [The tree](#the-tree).
- **Everything is synchronous.** There is no async plugin API and no
  `LazyResult`; `Processor::process` does the work and returns a result.
- **No custom-syntax plugging.** `opts.parser`/`stringifier`/`syntax` are not
  implemented. A custom syntax in Rust would build a [`Tree`] directly and write
  through the [`Build`] trait.
- **`Stringifier` is not subclassable.** Two upstream tests that override
  `raw()` or `rule()` in a subclass have no counterpart.
- **A processor with no plugins still parses.** PostCSS has a `NoWorkResult`
  fast path that skips parsing entirely and emits a single 1:1 source mapping;
  this crate always parses, so it produces a full map. Output CSS is unchanged.
- **The raw-style cache is per-call.** JS caches inferred whitespace on the root
  object, where it can go stale between `toString()` calls after a mutation.
- **`Container#push` is `push_child_public`,** since `push` would be misread as
  the normalizing `append`.
- **Regex-filtered walkers are absent** (`walkDecls(/^--/, …)`); filter inside
  the callback, or use the `_with_prop` / `_with_selector` / `_with_name`
  variants.

## Verifying it

Three layers, all runnable:

```sh
cargo test                      # 205 tests, no network or node required
cargo clippy --all-targets      # clean
```

The Rust suite includes:

- `tests/parse_fixtures.rs` — all 62 cases from
  [`postcss-parser-tests`](https://github.com/postcss/postcss-parser-tests), the
  suite PostCSS itself uses, comparing the full AST against the expected JSON and
  checking that stringifying reproduces the input byte for byte.
- `tests/tokenize.rs` — a port of upstream `tokenize.test.js`.
- `tests/stringifier.rs` — a port of upstream `stringifier.test.js`.
- `tests/tree.rs` — a port of upstream `container.test.ts` and `node.test.ts`.
- `tests/edge_cases.rs` — 49 adversarial inputs (unclosed constructs, IE hacks,
  escapes, custom properties with blocks, CRLF, non-ASCII, modern selectors,
  minified single-line output) checked against expectations recorded from
  PostCSS 8.5.23.
- `tests/source_map.rs`, `tests/plugins.rs`.

Differential testing against a live PostCSS (needs node):

```sh
cd tools && npm install && cd ..
cargo build --release --example ast_json --example process_map
node tools/diff-postcss.mjs tests/fixtures/cases tests/fixtures/edge   # AST + output + errors
node tools/diff-maps.mjs                                              # 72 map option combinations
```

`diff-postcss.mjs` accepts any files or directories, which is how this port was
checked against 476k lines of real-world CSS (Bootstrap, Bulma, Foundation, Pico,
Animate.css, normalize.css — 245 files, all matching exactly, including every
`raws` field and source offset).

### Against a real build pipeline

Public CSS files are hand-written and multi-line. What a bundler feeds PostCSS is
neither, so four more harnesses point at an actual app — anything with a
`postcss.config.*` — and use *that* app's PostCSS version:

```sh
export APP_DIR=/path/to/app

# Run the app's real plugin pipeline one plugin at a time, dumping the CSS and
# map each stage produced.
node tools/stage-capture.mjs /tmp/stages "$APP_DIR/src/entry.css"

# Every intermediate state through both implementations: AST, raws, offsets.
node tools/diff-postcss.mjs /tmp/stages

# Map chaining against those real maps, in the option shapes Vite asks for.
node tools/real-map-diff.mjs /tmp/stages

# A plugin and a Rust port of the same plugin, compared byte for byte.
JS_PLUGIN="$APP_DIR/path/to/plugin.cjs" \
  PLUGIN_BIN=target/release/examples/fnf_viewport_downlevel \
  node tools/plugin-diff.mjs /tmp/stages/*.css

# Where that pipeline's time actually goes, per plugin.
node tools/stage-timing.mjs "$APP_DIR/src/entry.css"
```

`examples/fnf_viewport_downlevel.rs` is the port of one such plugin, kept as a
worked example of writing a non-trivial plugin against this API — and as the
thing `plugin-diff.mjs` compares.

Run against a Tailwind v4 + `postcss-nesting` + `postcss-preset-env` app
(18.6 MiB of CSS across 408 files: hand-written sources, every pipeline stage, and
the minified bundles), all three diffs matched the JS implementation exactly.
`stage-timing.mjs` on that app is the sober counterweight: parse and stringify
are 25% of its CSS pipeline, the plugins' own JS is the other 75%. Replacing the
core alone moves the total by under 10%.

## Performance

`parse` + `to_css` over one file, mean of N iterations, Apple M-series, release
build vs. PostCSS on node 22:

| File | Rust parse | JS parse | Rust stringify | JS stringify | Rust total | JS total |
| --- | --- | --- | --- | --- | --- | --- |
| bootstrap.css (273 KiB) | 3.26 ms | 5.86 ms | 1.68 ms | 1.54 ms | **4.94 ms** | 7.40 ms |
| bulma.css (746 KiB) | 6.88 ms | 16.33 ms | 2.82 ms | 3.08 ms | **9.70 ms** | 19.41 ms |
| Tailwind v4 output (454 KiB, one line) | 5.11 ms | 8.56 ms | 2.14 ms | 2.40 ms | **7.25 ms** | 10.96 ms |
| bundled app CSS (3.2 MiB, minified) | 36.4 ms | 61.6 ms | 12.9 ms | 17.5 ms | **49.3 ms** | 79.0 ms |
| ditto, downleveled (6.8 MiB, minified) | 70.8 ms | 234.8 ms | 15.8 ms | 45.0 ms | **86.6 ms** | 279.9 ms |

Parsing is 1.6–3.3× faster; stringifying ranges from par to 2.8× faster. Total is
1.5–3.2× faster.

Minified CSS — one very long line — is worth calling out, because it is what a
build pipeline actually hands PostCSS and it used to be this port's worst case.
`column` counts characters, so computing one meant scanning from the start of the
line: fine at 80 columns, quadratic at 465,000. A 454 KiB Tailwind bundle parsed
in 259 ms, 34× *slower* than JS. [`Input`](src/input.rs) now keeps a table of
UTF-8 continuation-byte counts (skipped entirely for all-ASCII input), making the
lookup constant-time: the same file parses in 5.11 ms, and a 6.8 MiB one went
from 8.1 s to 71 ms.

Reproduce with:

```sh
cargo run --release --example bench -- path/to.css 30
node tools/bench-postcss.mjs path/to.css 30
```

## License

MIT, as PostCSS. Test fixtures under `tests/fixtures/cases` are copied from
`postcss-parser-tests` (MIT; see
`tests/fixtures/LICENSE-postcss-parser-tests`).

[`Tree`]: src/tree.rs
[`NodeId`]: src/node.rs
[`NewNode`]: src/node.rs
[`Insertable`]: src/tree.rs
[`Plugin`]: src/processor.rs
[`PluginContext`]: src/result.rs
[`Build`]: src/stringifier.rs
