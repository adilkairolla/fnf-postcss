# fnf-postcss

PostCSS with a Rust core. Same API, same plugins, parsing and source maps in
native code.

```sh
npm install fnf-postcss
```

```js
const postcss = require('fnf-postcss')
const autoprefixer = require('autoprefixer')

const result = await postcss([autoprefixer]).process(css, {
  from: 'src/app.css',
  map: { inline: false },
  to: 'dist/app.css'
})
```

Or drop it in where `postcss` already is:

```js
// vite.config.js
import postcss from 'fnf-postcss'
// webpack, next.config.js, postcss.config.js — anywhere postcss is imported
```

## What is native and what is not

Honest breakdown, because it decides whether this is worth it for you:

| Part | Implementation |
| --- | --- |
| Tokenizer, parser | **Rust** |
| Source map generation and chaining | **Rust** |
| `postcss.stringify` on an unmutated tree | **Rust** |
| `list.comma` / `list.space` | **Rust** |
| Node classes, walkers, mutation, plugin pipeline | PostCSS's own JS, included under its MIT licence |

Plugins get PostCSS's actual object model — `Root`, `Rule`, `Declaration`,
`walkDecls`, `raws`, visitors, warnings, `CssSyntaxError` — because that layer
*is* PostCSS's, vendored in `lib/` and attributed at the top of each file. A
hand-written lookalike would have been a compatibility risk for no benefit. What
sits underneath it is the Rust core from
[the parent repository](https://github.com/adilkairolla/fnf-postcss).

So: your plugins keep running unchanged, and the parse and source-map work — the
expensive, mechanical half — happens in native code.

## Is it faster?

Parsing is 1.6–3.3× faster than PostCSS on the same file, most of all on the
minified, single-line CSS a bundler actually produces:

| File | Rust parse | JS parse |
| --- | --- | --- |
| bootstrap.css (273 KiB) | 3.3 ms | 5.9 ms |
| Tailwind v4 output (454 KiB, one line) | 5.1 ms | 8.6 ms |
| bundled app CSS (3.2 MiB, minified) | 36 ms | 62 ms |
| ditto, downleveled (6.8 MiB, minified) | 71 ms | 235 ms |

But measure your own pipeline before expecting a big win. On a real Tailwind v4 +
`postcss-nesting` + `postcss-preset-env` build, parse and stringify are **25% of
the CSS pipeline** and the plugins' own JavaScript is the other 75% — that part
is unchanged here. Replacing the core alone moved that build by under 10%. The
gain is larger the fewer plugins you run, and largest if all you do is
parse/stringify.

## Compatibility

Checked by running real, unmodified plugins through both this package and
`postcss` over the same CSS, then comparing the output CSS, the source map and
the warnings byte for byte:

- A production Tailwind v4 → `postcss-nesting` → downlevel → `postcss-preset-env`
  pipeline over 293 CSS files × 3 source-map modes: **879/879 identical**.
- The 62 `postcss-parser-tests` cases and 49 adversarial inputs through a plugin
  set that clones, wraps, reorders and warns: identical, except as noted below.

Run it yourself:

```sh
node test/drop-in.mjs path/to/css              # built-in plugin set
APP_DIR=/path/to/app node test/drop-in.mjs …   # your app's real pipeline
```

### One known difference

Source-map columns for lines containing **non-ASCII** characters can differ by a
few positions. The Rust core counts characters where PostCSS counts UTF-16 code
units; they agree for ASCII, which is all of CSS's own syntax, and diverge only
inside non-ASCII identifiers, strings and comments. Output CSS is always
identical. If your CSS is ASCII — most is; a 3.8 MiB bundle we tested had three
non-ASCII bytes — this cannot affect you.

Not implemented: `opts.parser` / `opts.stringifier` / `opts.syntax`. A custom
syntax means a different parser, which is the one thing this package replaces.

`result.processor.version` reports `8.5.23`, the PostCSS version this tracks,
because plugins read it to decide which API they are talking to. The package's
own version is in `package.json`.

## Native binary

The package bundles one prebuilt addon, for **macOS arm64** (Apple silicon) —
built and published from a developer machine rather than from CI, so that is the
platform covered today. Anywhere else, compile one in place; it takes a few
seconds and needs only a [Rust toolchain](https://rustup.rs):

```sh
npm run build --prefix node_modules/fnf-postcss
```

That writes the addon next to the loader under the same name it looks for, so
nothing else needs configuring. `npm run smoke --prefix node_modules/fnf-postcss`
then checks it parses, mutates, generates a map and reports errors correctly.

The addon is also usable directly, without the JS layer:

```js
const native = require('fnf-postcss/native')
native.parse(css, { from: 'a.css' })     // AST as plain objects
native.process(css, { from, to, map: true })
```

## License

MIT. Includes PostCSS's JS object model (MIT, Copyright 2013 Andrey Sitnik) — see
LICENSE and the header of each file in `lib/`.
