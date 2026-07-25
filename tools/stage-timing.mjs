// Where an app's CSS build time actually goes: per plugin, and within each
// plugin how much is PostCSS parse/stringify (which this crate could replace)
// versus the plugin's own JS work (which it could not).
//
//   APP_DIR=/path/to/app node stage-timing.mjs <entry.css> [iterations]

import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join } from 'node:path'

// PostCSS is resolved from the app under test (`APP_DIR`), so the comparison
// runs against the exact version that app's build uses, with its plugins
// resolvable. Falls back to this repo's tools/node_modules copy.
const require_ = createRequire(
  process.env.APP_DIR ? join(process.env.APP_DIR, 'package.json') : import.meta.url
)
const postcss = require_('postcss')
const postcssrc = require_('postcss-load-config')

const APP = process.env.APP_DIR ?? process.cwd()
const [entry, count = '5'] = process.argv.slice(2)
const iterations = Number(count)

process.env.NODE_ENV = 'production'
process.env.POSTCSS_FULL = '1'

const { plugins } = await postcssrc({ env: 'production' }, APP)
const source = readFileSync(entry, 'utf8')

const totals = []

for (const [index, plugin] of plugins.entries()) {
  const name = plugin.postcssPlugin ?? `plugin${index}`
  totals.push({ name, parse: 0, plugin: 0, stringify: 0 })
}

let stageInput = source
const stageInputs = []

// One pass to collect each stage's input.
for (const [index, plugin] of plugins.entries()) {
  stageInputs[index] = stageInput
  stageInput = (await postcss([plugin]).process(stageInput, { from: entry, map: false })).css
}

for (let round = 0; round < iterations; round++) {
  for (const [index, plugin] of plugins.entries()) {
    const css = stageInputs[index]
    const entryTotals = totals[index]

    // Parse alone.
    let start = performance.now()
    const root = postcss.parse(css, { from: entry })
    entryTotals.parse += performance.now() - start

    // The plugin's own work, on an already-parsed tree.
    const runner = postcss([plugin])
    start = performance.now()
    const result = await runner.process(root, { from: entry, map: false })
    // Force the transform without stringifying yet.
    void result.root
    entryTotals.plugin += performance.now() - start

    // Stringify alone.
    start = performance.now()
    const out = result.root.toString()
    entryTotals.stringify += performance.now() - start
    if (!out) throw new Error('empty')
  }
}

console.log(`${entry}  (${(Buffer.byteLength(source) / 1024).toFixed(0)} KiB source, ${iterations} iterations)\n`)
console.log('stage                    parse   plugin work   stringify      total')
let grandParse = 0
let grandPlugin = 0
let grandStringify = 0
for (const t of totals) {
  const parse = t.parse / iterations
  const work = t.plugin / iterations
  const stringify = t.stringify / iterations
  grandParse += parse
  grandPlugin += work
  grandStringify += stringify
  console.log(
    `${t.name.padEnd(22)} ${parse.toFixed(1).padStart(7)} ms ${work.toFixed(1).padStart(9)} ms ` +
      `${stringify.toFixed(1).padStart(9)} ms ${(parse + work + stringify).toFixed(1).padStart(8)} ms`
  )
}
const grand = grandParse + grandPlugin + grandStringify
console.log(
  `\n${'TOTAL'.padEnd(22)} ${grandParse.toFixed(1).padStart(7)} ms ${grandPlugin.toFixed(1).padStart(9)} ms ` +
    `${grandStringify.toFixed(1).padStart(9)} ms ${grand.toFixed(1).padStart(8)} ms`
)
console.log(
  `\nparse + stringify = ${(((grandParse + grandStringify) / grand) * 100).toFixed(1)}% of the pipeline; ` +
    `plugin JS = ${((grandPlugin / grand) * 100).toFixed(1)}%`
)
