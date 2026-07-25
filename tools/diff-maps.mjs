// Differential test for source map generation.
//
// Runs a matrix of map options through the JS PostCSS and through this crate,
// comparing the output CSS and the generated map.
//
// Usage:
//   node tools/diff-maps.mjs

import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import postcss from 'postcss'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const BIN = process.env.PROCESS_MAP_BIN ?? join(ROOT, 'target/release/examples/process_map')

const INPUTS = {
  simple: 'a { color: black }\n',
  nested: '@media screen {\n  a {\n    color: black;\n    top: 0;\n  }\n}\n',
  comments: '/* c */\na,\nb {\n  color: black;\n}\n/* end */\n',
  multiline: 'a {\n  background:\n    url(a.png)\n    no-repeat;\n}\nb { top: 0 }\n',
  crlf: 'a {\r\n  color: black;\r\n}\r\n',
  emptyRule: 'a {}\nb { top: 0 }\n',
  atRuleNoBlock: '@charset "utf-8";\na { top: 0 }\n',
  important: 'a { color: red !important }\n',
  customProps: ':root { --x: 1px }\na { top: var(--x) }\n'
}

// A previous map, so chaining is covered too.
const PREV = JSON.stringify({
  version: 3,
  file: 'a.css',
  sources: ['a.scss'],
  sourcesContent: ['a\n  color: black\n'],
  names: [],
  mappings: 'AAAA;EACE'
})

const CASES = []
for (const [name, css] of Object.entries(INPUTS)) {
  CASES.push({ name: `${name}/inline`, css, opts: { from: 'a.css', to: 'b.css', map: { inline: true } } })
  CASES.push({ name: `${name}/external`, css, opts: { from: 'a.css', to: 'b.css', map: { inline: false } } })
  CASES.push({
    name: `${name}/no-sources-content`,
    css,
    opts: { from: 'a.css', to: 'b.css', map: { inline: false, sourcesContent: false } }
  })
  CASES.push({
    name: `${name}/annotation-path`,
    css,
    opts: { from: 'a.css', to: 'b.css', map: { annotation: 'maps/b.css.map', inline: false } }
  })
  CASES.push({
    name: `${name}/no-annotation`,
    css,
    opts: { from: 'a.css', to: 'b.css', map: { annotation: false, inline: false } }
  })
  CASES.push({
    name: `${name}/nested-dirs`,
    css,
    opts: { from: 'src/a.css', to: 'dist/css/b.css', map: { inline: false } }
  })
  CASES.push({
    name: `${name}/map-from`,
    css,
    opts: { from: 'a.css', to: 'b.css', map: { inline: false, from: 'webpack://./a.css' } }
  })
  CASES.push({
    name: `${name}/prev`,
    css,
    opts: { from: 'a.css', to: 'b.css', map: { inline: false, prev: PREV } }
  })
}

// With no plugins, PostCSS takes its `NoWorkResult` path: it never parses, and
// emits a single 1:1 mapping. A no-op plugin forces the real pipeline, which is
// what this crate always runs.
const NOOP = { Once() {}, postcssPlugin: 'noop' }

function withJs({ css, opts }) {
  const result = postcss([NOOP]).process(css, opts)
  return {
    css: result.css,
    map: result.map ? JSON.parse(JSON.stringify(result.map.toJSON())) : null
  }
}

function withRust({ css, opts }) {
  const args = []
  if (opts.from) args.push('--from', opts.from)
  if (opts.to) args.push('--to', opts.to)
  const map = opts.map ?? {}
  if (typeof map.inline === 'boolean') args.push('--inline', String(map.inline))
  if (typeof map.sourcesContent === 'boolean') {
    args.push('--sources-content', String(map.sourcesContent))
  }
  if (map.annotation !== undefined) args.push('--annotation', String(map.annotation))
  if (map.from) args.push('--map-from', map.from)
  if (map.absolute) args.push('--absolute', 'true')
  if (map.prev) args.push('--prev', map.prev)

  const out = execFileSync(BIN, args, { input: css, maxBuffer: 1 << 28 })
  return JSON.parse(out.toString())
}

function diff(a, b, path = '') {
  if (a === b) return null
  if (typeof a !== typeof b || a === null || b === null || typeof a !== 'object') {
    return `${path || '<root>'}: ${JSON.stringify(a)} !== ${JSON.stringify(b)}`
  }
  if (Array.isArray(a)) {
    if (a.length !== b.length) return `${path}.length: ${a.length} !== ${b.length}`
    for (let i = 0; i < a.length; i++) {
      const found = diff(a[i], b[i], `${path}[${i}]`)
      if (found) return found
    }
    return null
  }
  for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) {
    // `version` is a number in both; ignore key order only.
    const found = diff(a[key], b[key], `${path}.${key}`)
    if (found) return found
  }
  return null
}

let failed = 0
for (const testCase of CASES) {
  const js = withJs(testCase)
  const rust = withRust(testCase)
  const found = diff(js, rust)
  if (found) {
    failed++
    console.log(`\x1b[31mFAIL\x1b[0m ${testCase.name}`)
    console.log(`  ${found}`)
  }
}

console.log(`\n${CASES.length - failed}/${CASES.length} map cases matched the JS implementation`)
process.exit(failed === 0 ? 0 : 1)
