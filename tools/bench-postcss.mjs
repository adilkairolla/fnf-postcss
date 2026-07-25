// Times parse and stringify through the JS PostCSS, matching
// `examples/bench.rs`, so the two numbers are comparable.
//
// Usage:
//   node tools/bench-postcss.mjs path/to.css [iterations]

import { readFileSync } from 'node:fs'

import postcss from 'postcss'

const [path, count = '20'] = process.argv.slice(2)
if (!path) {
  console.error('usage: node tools/bench-postcss.mjs <file.css> [iterations]')
  process.exit(2)
}

const iterations = Number(count)
const css = readFileSync(path, 'utf8')
const opts = { from: path, map: false }

// Warm up, and fail loudly before timing if the file does not parse.
const warm = postcss.parse(css, opts)
if (warm.toString() !== css) throw new Error('round-trip is not byte-exact')

let parseTotal = 0
let stringifyTotal = 0

for (let i = 0; i < iterations; i++) {
  let start = performance.now()
  const root = postcss.parse(css, opts)
  parseTotal += performance.now() - start

  start = performance.now()
  const out = root.toString()
  stringifyTotal += performance.now() - start
  if (!out) throw new Error('empty output')
}

const parse = parseTotal / iterations
const stringify = stringifyTotal / iterations
console.log(`${path}: ${Math.round(css.length / 1024)} KiB, ${iterations} iterations`)
console.log(`  parse:     ${parse.toFixed(2).padStart(8)} ms`)
console.log(`  stringify: ${stringify.toFixed(2).padStart(8)} ms`)
console.log(`  total:     ${(parse + stringify).toFixed(2).padStart(8)} ms`)
