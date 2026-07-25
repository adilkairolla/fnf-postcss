// Differential test: parses each input with the JS PostCSS and with this crate,
// then compares the AST, the stringified output, and the error position.
//
// Usage:
//   node tools/diff-postcss.mjs <file-or-dir>...
//
// Requires `postcss` to be resolvable (see tools/package.json) and
// `cargo build --release --example ast_json` to have run.

import { execFileSync } from 'node:child_process'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import postcss from 'postcss'

// Resolve relative to this file, so the script works from any directory.
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const BIN = process.env.AST_JSON_BIN ?? join(ROOT, 'target/release/examples/ast_json')

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

// This crate reports `offset` as a UTF-8 byte offset and `column` as a
// character count; JS counts UTF-16 code units for both. Rewrite the JS
// positions into the Rust model so the diff still checks them exactly.
function toRustPositions(value, css) {
  // For ASCII input the three models coincide, so there is nothing to convert.
  // Worth special-casing: a per-node `css.slice(0, offset)` is O(n) each, which
  // on a multi-megabyte minified bundle is quadratic and never finishes.
  // eslint-disable-next-line no-control-regex
  if (!/[^\x00-\x7f]/.test(css)) return value

  // Three tables, built in one pass over the input, all indexed by UTF-16 index:
  // the byte offset, the character count since the start of the line, and where
  // each line begins. Everything below is then a lookup — the previous version
  // sliced the string per node, which is quadratic on a minified bundle.
  const byteOffset = new Int32Array(css.length + 1)
  const charsInLine = new Int32Array(css.length + 1)
  const lineStart = [0]

  for (let index = 0; index < css.length; index++) {
    const code = css.codePointAt(index)
    const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4
    const wide = code > 0xffff

    byteOffset[index + 1] = byteOffset[index] + width
    charsInLine[index + 1] = charsInLine[index] + 1
    if (wide) {
      // A surrogate pair is two UTF-16 units but one character of 4 bytes; the
      // trailing unit adds neither a byte nor a character.
      byteOffset[index + 2] = byteOffset[index + 1]
      charsInLine[index + 2] = charsInLine[index + 1]
      index++
    }
    if (code === 10) {
      lineStart.push(index + 1)
      charsInLine[index + 1] = 0
    }
  }

  const convert = position => {
    if (!position) return position
    if (typeof position.offset === 'number') {
      position.offset = byteOffset[Math.min(position.offset, css.length)]
    }
    if (typeof position.line === 'number' && typeof position.column === 'number') {
      const start = lineStart[position.line - 1] ?? 0
      const index = Math.min(start + position.column - 1, css.length)
      // `\r` is stripped from the line in the Rust model, so a column pointing
      // at a CRLF terminator stays where it is.
      position.column = charsInLine[index] + 1
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

function withJs(css, from) {
  try {
    const root = postcss.parse(css, { from, map: false })
    return { ast: toRustPositions(jsonify(root), css), css: root.toString() }
  } catch (e) {
    if (e.name !== 'CssSyntaxError') throw e
    return {
      error: {
        reason: e.reason,
        line: e.line ?? null,
        column: e.column ?? null,
        endLine: e.endLine ?? null,
        endColumn: e.endColumn ?? null
      }
    }
  }
}

function withRust(css) {
  const out = execFileSync(BIN, [], { input: css, maxBuffer: 1 << 28 })
  return JSON.parse(out.toString())
}

// Compares two values, reporting the path of the first difference.
function diff(a, b, path = '') {
  if (a === b) return null
  if (typeof a !== typeof b || a === null || b === null) {
    return `${path || '<root>'}: ${JSON.stringify(a)} !== ${JSON.stringify(b)}`
  }
  if (typeof a !== 'object') {
    return `${path || '<root>'}: ${JSON.stringify(a)} !== ${JSON.stringify(b)}`
  }
  if (Array.isArray(a) !== Array.isArray(b)) {
    return `${path}: array vs object`
  }
  if (Array.isArray(a)) {
    if (a.length !== b.length) {
      return `${path}.length: ${a.length} !== ${b.length}`
    }
    for (let i = 0; i < a.length; i++) {
      const found = diff(a[i], b[i], `${path}[${i}]`)
      if (found) return found
    }
    return null
  }
  const keys = new Set([...Object.keys(a), ...Object.keys(b)])
  for (const key of keys) {
    if (!(key in a)) return `${path}.${key}: missing in JS, ${JSON.stringify(b[key])} in Rust`
    if (!(key in b)) return `${path}.${key}: ${JSON.stringify(a[key])} in JS, missing in Rust`
    const found = diff(a[key], b[key], `${path}.${key}`)
    if (found) return found
  }
  return null
}

function collectFiles(target) {
  const stat = statSync(target)
  if (stat.isFile()) return [target]
  return readdirSync(target)
    .filter(name => name.endsWith('.css'))
    .map(name => join(target, name))
}

const targets = process.argv.slice(2)
if (targets.length === 0) {
  console.error('usage: node tools/diff-postcss.mjs <file-or-dir>...')
  process.exit(2)
}

let checked = 0
let failed = 0

for (const target of targets) {
  for (const file of collectFiles(target)) {
    const css = readFileSync(file, 'utf8')
    checked++

    const js = withJs(css, file)
    const rust = withRust(css)

    const found = diff(js, rust)
    if (found) {
      failed++
      console.log(`\x1b[31mFAIL\x1b[0m ${file}`)
      console.log(`  ${found}`)
    }
  }
}

console.log(`\n${checked - failed}/${checked} inputs matched the JS implementation`)
process.exit(failed === 0 ? 0 : 1)
