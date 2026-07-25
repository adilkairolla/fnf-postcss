'use strict'

// Parsing is where the Rust core earns its place: it tokenizes and parses, and
// hands back the AST as plain JS objects (napi's serde bridge, so no JSON string
// is ever built). This module's job is to turn those plain objects into real
// `Root`/`Rule`/`AtRule`/`Declaration`/`Comment` instances and hang `Input`
// objects off their `source`, which is one pass and no re-parsing.

let AtRule = require('./at-rule')
let Comment = require('./comment')
let Container = require('./container')
let CssSyntaxError = require('./css-syntax-error')
let Declaration = require('./declaration')
let Input = require('./input')
let native = require('../index.js')
let Root = require('./root')
let Rule = require('./rule')
let { isClean, my } = require('./symbols')

const PROTOTYPES = {
  atrule: AtRule.prototype,
  comment: Comment.prototype,
  decl: Declaration.prototype,
  root: Root.prototype,
  rule: Rule.prototype
}

// A syntax error crosses the boundary as a JSON payload in the message, since
// that is all a napi error carries. Rebuild the real thing from it.
function rethrow(error, css, opts) {
  let payload
  try {
    payload = JSON.parse(error.message)
  } catch {
    throw error
  }
  if (!payload || !payload.__cssSyntaxError) throw error

  // The constructor takes either two numbers (a start only) or two position
  // objects (a range) — and ignores both unless *both* arguments are defined, so
  // a start with no end has to be passed as numbers.
  let start
  let end
  if (payload.line != null && payload.endLine != null) {
    start = { column: payload.column, line: payload.line }
    end = { column: payload.endColumn, line: payload.endLine }
  } else if (payload.line != null) {
    start = payload.line
    end = payload.column
  }

  let rebuilt = new CssSyntaxError(
    payload.reason,
    start,
    end,
    payload.source ?? css,
    payload.file ?? (opts && opts.from),
    payload.plugin ?? undefined
  )
  if (payload.input) {
    rebuilt.input = { ...payload.input, source: payload.source ?? css }
  }
  throw rebuilt
}

// The AST carries one `inputs` array and every `source` points into it by
// index, so identical inputs stay shared rather than duplicated per node.
function buildInputs(ast, css, opts) {
  let inputs = ast.inputs
  delete ast.inputs
  if (!inputs || inputs.length === 0) return [new Input(css, opts)]
  return inputs.map(json => {
    let input = new Input(json.css ?? css, opts)
    if (json.file) input.file = json.file
    if (json.id) input.id = json.id
    if (json.hasBOM) input.hasBOM = json.hasBOM
    return input
  })
}

module.exports = function parse(css, opts) {
  let text = css == null ? '' : String(css)

  let ast
  try {
    ast = native.parse(text, {
      from: opts && opts.from ? String(opts.from) : undefined,
      // Map discovery on the Rust side would read the filesystem looking for a
      // neighbouring map; `Input` does that itself, on the JS side.
      map: false
    })
  } catch (error) {
    rethrow(error, text, opts)
  }

  let inputs = buildInputs(ast, text, opts)

  // One iterative pass: give every node its prototype, its parent, and an
  // `Input` in place of the `inputId` index.
  let stack = [[ast, undefined]]
  while (stack.length > 0) {
    let [node, parent] = stack.pop()

    let prototype = PROTOTYPES[node.type]
    /* c8 ignore next 3 */
    if (!prototype) {
      throw new Error('Unknown AST node type ' + node.type)
    }
    Object.setPrototypeOf(node, prototype)
    node[my] = true
    node[isClean] = false
    if (parent) node.parent = parent
    if (!node.raws) node.raws = {}

    if (node.source) {
      let input = inputs[node.source.inputId] || inputs[0]
      delete node.source.inputId
      node.source.input = input
    }

    if (node.nodes) {
      for (let index = node.nodes.length - 1; index >= 0; index--) {
        stack.push([node.nodes[index], node])
      }
    }
  }

  // `Root` needs the input on itself too, for `toResult` and error reporting.
  if (!ast.source) ast.source = { input: inputs[0] }

  return ast
}

// `Container#normalize` parses CSS strings through whatever parser is
// registered; point it at this one so `append('a{}')` uses the Rust parser too.
Container.registerParse(module.exports)
