// Records what the JS PostCSS produces for a corpus, in this crate's position
// model, so the Rust test suite can check itself without needing node.
//
// Usage:
//   node tools/snapshot-postcss.mjs <dir> <out.json>

import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'

import postcss from 'postcss'

function jsonify(node) {
  const clean = n => {
    if (n.source) {
      delete n.source.input
      delete n.source.inputId
    }
    delete n.inputs
    delete n.indexes
    delete n.lastEach
    delete n.rawCache
    if (n.nodes) n.nodes = n.nodes.map(clean)
    return n
  }
  return JSON.parse(JSON.stringify(clean(node.toJSON()), null, 2))
}

// Rust reports byte offsets and character columns; JS reports UTF-16 units.
function toRustPositions(value, css) {
  const lines = css.split('\n').map(line => line.replace(/\r$/, ''))
  const convert = position => {
    if (!position) return position
    if (typeof position.offset === 'number') {
      position.offset = Buffer.byteLength(css.slice(0, position.offset), 'utf8')
    }
    if (typeof position.line === 'number' && typeof position.column === 'number') {
      const line = lines[position.line - 1] ?? ''
      position.column = [...line.slice(0, position.column - 1)].length + 1
    }
    return position
  }
  const walk = node => {
    if (node.source) {
      convert(node.source.start)
      convert(node.source.end)
    }
    if (node.nodes) node.nodes.forEach(walk)
    return node
  }
  return walk(value)
}

const [dir, out] = process.argv.slice(2)
if (!dir || !out) {
  console.error('usage: node tools/snapshot-postcss.mjs <dir> <out.json>')
  process.exit(2)
}

const snapshot = {}
const files = readdirSync(dir)
  .filter(name => name.endsWith('.css'))
  .sort()

for (const name of files) {
  const css = readFileSync(join(dir, name), 'utf8')
  try {
    const root = postcss.parse(css, { from: name, map: false })
    snapshot[basename(name)] = {
      ast: toRustPositions(jsonify(root), css),
      css: root.toString()
    }
  } catch (e) {
    if (e.name !== 'CssSyntaxError') throw e
    const lines = css.split('\n').map(line => line.replace(/\r$/, ''))
    const column = (line, col) =>
      col == null ? null : [...(lines[line - 1] ?? '').slice(0, col - 1)].length + 1
    snapshot[basename(name)] = {
      error: {
        reason: e.reason,
        line: e.line ?? null,
        column: column(e.line, e.column),
        endLine: e.endLine ?? null,
        endColumn: column(e.endLine, e.endColumn)
      }
    }
  }
}

writeFileSync(out, JSON.stringify(snapshot, null, 2) + '\n')
console.log(`wrote ${Object.keys(snapshot).length} cases to ${out}`)
console.log(`postcss ${JSON.parse(readFileSync(new URL('./node_modules/postcss/package.json', import.meta.url))).version}`)
