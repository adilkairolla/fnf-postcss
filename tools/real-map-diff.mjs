// Source-map differential test on real maps, not synthetic ones.
//
// `stage-capture.mjs` leaves a directory of CSS files each paired with the map
// PostCSS produced for it. Feed every (css, prev-map) pair through JS PostCSS
// and through this crate with the map options Vite asks for, and compare the
// resulting map field by field. That exercises chaining against real
// hundred-KiB maps with thousands of mappings, which `diff-maps.mjs` does not.
//
//   node real-map-diff.mjs <stages-dir>

import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
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
const BIN = process.env.PROCESS_MAP_BIN ?? join(ROOT, 'target/release/examples/process_map')

const dir = process.argv[2]
if (!dir) {
  console.error('usage: node real-map-diff.mjs <stages-dir>')
  process.exit(2)
}

// The option shapes a Vite build actually asks PostCSS for.
const VARIANTS = [
  { name: 'vite-dev-inline', opts: { inline: true, sourcesContent: true } },
  { name: 'vite-build-external', opts: { annotation: false, inline: false, sourcesContent: true } },
  { name: 'no-sources-content', opts: { annotation: false, inline: false, sourcesContent: false } },
  { name: 'annotated', opts: { inline: false, sourcesContent: true } }
]

function rustRun(css, from, to, variant, prev) {
  const args = ['--from', from, '--to', to]
  args.push('--inline', String(variant.opts.inline ?? true))
  if (variant.opts.sourcesContent !== undefined) {
    args.push('--sources-content', String(variant.opts.sourcesContent))
  }
  if (variant.opts.annotation !== undefined) {
    args.push('--annotation', String(variant.opts.annotation))
  }
  if (prev) args.push('--prev', prev)
  return JSON.parse(execFileSync(BIN, args, { input: css, maxBuffer: 1 << 28 }).toString())
}

async function jsRun(css, from, to, variant, prev) {
  // A no-op plugin, so PostCSS takes the real path rather than NoWorkResult.
  const noop = { postcssPlugin: 'noop', Once() {} }
  const result = await postcss([noop]).process(css, {
    from,
    map: { ...variant.opts, prev },
    to
  })
  return { css: result.css, map: result.map ? JSON.parse(result.map.toString()) : null }
}

// Compares two maps, reporting the first difference. `mappings` is compared as a
// whole string: a single differing segment is a real difference.
function diffMap(js, rust, path = '') {
  if (js === rust) return null
  if (typeof js !== typeof rust || js === null || rust === null || typeof js !== 'object') {
    const show = v => {
      const s = JSON.stringify(v) ?? String(v)
      return s.length > 160 ? `${s.slice(0, 160)}…(${s.length} chars)` : s
    }
    return `${path || '<root>'}: JS ${show(js)} !== Rust ${show(rust)}`
  }
  if (Array.isArray(js)) {
    if (js.length !== rust.length) return `${path}.length: ${js.length} !== ${rust.length}`
    for (let i = 0; i < js.length; i++) {
      const found = diffMap(js[i], rust[i], `${path}[${i}]`)
      if (found) return found
    }
    return null
  }
  for (const key of new Set([...Object.keys(js), ...Object.keys(rust)])) {
    if (!(key in js)) return `${path}.${key}: missing in JS`
    if (!(key in rust)) return `${path}.${key}: missing in Rust`
    const found = diffMap(js[key], rust[key], `${path}.${key}`)
    if (found) return found
  }
  return null
}

const files = readdirSync(dir)
  .filter(name => name.endsWith('.css'))
  .sort()

let checked = 0
let failed = 0

for (const name of files) {
  const file = join(dir, name)
  const css = readFileSync(file, 'utf8')
  const mapFile = `${file}.map`
  const prev = existsSync(mapFile) ? readFileSync(mapFile, 'utf8') : undefined

  for (const variant of VARIANTS) {
    checked++
    const from = process.env.MAP_FROM ?? 'src/entry.css'
    const to = process.env.MAP_TO ?? 'dist/assets/out.css'
    const t0 = performance.now()
    const js = await jsRun(css, from, to, variant, prev)
    const rust = rustRun(css, from, to, variant, prev)

    let found = js.css === rust.css ? null : 'css output differs'
    if (!found) found = diffMap(js.map, rust.map, 'map')
    const ms = (performance.now() - t0).toFixed(0)

    if (found) {
      failed++
      console.log(`\x1b[31mFAIL\x1b[0m ${name} ${variant.name}\n  ${found}`)
    } else {
      const mappings = js.map?.mappings?.length ?? 0
      console.log(
        `  ok ${name.replace(/^.*index\.css\./, '')} ${variant.name.padEnd(21)} ` +
          `${String(mappings).padStart(7)} mapping chars, prev ${prev ? 'yes' : 'no '}, ${ms.padStart(6)} ms`
      )
    }
  }
}

console.log(`\n${checked - failed}/${checked} map cases matched the JS implementation`)
process.exit(failed === 0 ? 0 : 1)
