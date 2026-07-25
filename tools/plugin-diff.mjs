// Runs a PostCSS plugin and a Rust port of that same plugin over every CSS file
// given, and compares the output byte for byte. Where diff-postcss checks
// parse/stringify, this checks the plugin layer: clone, removeAll, append,
// insertAfter, walkRules/walkDecls, OnceExit.
//
//   JS_PLUGIN=/path/to/plugin.cjs PLUGIN_BIN=target/release/examples/my_port \
//     node plugin-diff.mjs <file>...

import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

// PostCSS is resolved from the app under test (`APP_DIR`), so the comparison
// runs against the exact version that app's build uses, with its plugins
// resolvable. Falls back to this repo's tools/node_modules copy.
const require_ = createRequire(
  process.env.APP_DIR ? join(process.env.APP_DIR, 'package.json') : import.meta.url
)
const postcss = require_('postcss')

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
// The Rust port of the plugin, and the JS original to compare it against.
const BIN = process.env.PLUGIN_BIN ?? join(ROOT, 'target/release/examples/fnf_viewport_downlevel')
const JS_PLUGIN = process.env.JS_PLUGIN
if (!JS_PLUGIN) {
  console.error('set JS_PLUGIN to the .cjs plugin to compare against')
  process.exit(2)
}

const plugin = (await import(JS_PLUGIN)).default
const files = process.argv.slice(2)

let checked = 0
let failed = 0
let transformed = 0

function firstDifference(a, b) {
  const limit = Math.min(a.length, b.length)
  let index = 0
  while (index < limit && a[index] === b[index]) index++
  const context = 90
  return (
    `at byte ${index} (JS ${a.length} bytes, Rust ${b.length} bytes)\n` +
    `    JS   …${JSON.stringify(a.slice(Math.max(0, index - 40), index + context))}\n` +
    `    Rust …${JSON.stringify(b.slice(Math.max(0, index - 40), index + context))}`
  )
}

for (const file of files) {
  const css = readFileSync(file, 'utf8')
  checked++

  let js
  try {
    js = (await postcss([plugin()]).process(css, { from: file, map: false })).css
  } catch (e) {
    console.log(`\x1b[33mskip\x1b[0m ${file}: JS plugin threw ${e.name}`)
    continue
  }
  const rust = execFileSync(BIN, [file], { input: css, maxBuffer: 1 << 28 }).toString()

  if (js !== css) transformed++

  if (js !== rust) {
    failed++
    console.log(`\x1b[31mFAIL\x1b[0m ${file}\n  ${firstDifference(js, rust)}`)
  }
}

console.log(
  `\n${checked - failed}/${checked} files matched byte for byte ` +
    `(${transformed} were actually transformed by the plugin)`
)
process.exit(failed === 0 ? 0 : 1)
