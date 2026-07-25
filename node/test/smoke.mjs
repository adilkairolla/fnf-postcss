// Proves a freshly built addon actually loads and works on this machine.
//
// Deliberately small and dependency-free: the release workflow runs it on every
// platform, including inside an Alpine container for the musl builds, where only
// node and this repository are available.
//
//   node test/smoke.mjs

import { createRequire } from 'node:module'

const require_ = createRequire(import.meta.url)
const postcss = require_('../lib/postcss.js')
const native = require_('../index.js')

function check(what, condition) {
  if (!condition) throw new Error(`smoke test failed: ${what}`)
  console.log(`ok  ${what}`)
}

// Round-trips byte for byte, which is the whole contract of the parser.
const css = 'a {\n  color: red;\n}\n/* c */\n@media print { b { top: 0 } }\n'
check('parse and stringify round-trip', postcss.parse(css).toString() === css)

// Mutation through the JS layer, then stringifying the changed tree.
const root = postcss.parse('a { color: red }')
root.walkDecls(decl => {
  decl.value = 'green'
})
check('mutation', root.toString() === 'a { color: green }')

// A plugin, a source map, and the plugin pipeline that drives both.
const plugin = {
  postcssPlugin: 'smoke',
  Declaration(decl) {
    if (decl.prop === 'color') decl.prop = '-x-color'
  }
}
const result = await postcss([plugin]).process('a { color: red }\n', {
  from: 'a.css',
  map: { inline: false },
  to: 'b.css'
})
check('plugin ran', result.css.includes('-x-color'))
check('map generated', JSON.parse(result.map.toString()).mappings.length > 0)

// Errors must arrive as real CssSyntaxErrors with positions, not as opaque
// failures from across the language boundary.
let error
try {
  postcss.parse('a {', { from: 'broken.css' })
} catch (thrown) {
  error = thrown
}
check('syntax error thrown', error && error.name === 'CssSyntaxError')
check('syntax error reason', error.reason === 'Unclosed block')
check('syntax error position', error.line === 1 && error.column === 1)

check('list.comma', postcss.list.comma('a, b(c, d)').length === 2)
check('core version', typeof native.coreVersion() === 'string')

console.log(`\nall good on ${process.platform}-${process.arch}, core ${native.coreVersion()}`)
