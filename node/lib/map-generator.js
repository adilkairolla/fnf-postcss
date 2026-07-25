'use strict'

// Stands in for PostCSS's `map-generator.js`. Generating a map means walking the
// whole tree recording where each node landed while stringifying it, and the
// Rust core already does exactly that — verified byte-identical to PostCSS,
// down to the base64 payload of an inline annotation. So this hands the tree
// over rather than doing the walk again in JS.

let native = require('../index.js')

class MapGenerator {
  constructor(stringify, root, opts, cssString) {
    this.stringify = stringify
    this.mapOpts = opts.map || {}
    this.root = root
    this.opts = opts
    this.css = cssString
  }

  generate() {
    if (!this.root) {
      // The no-plugin path: PostCSS hands the CSS string straight through.
      return [this.css, undefined]
    }

    let options = {
      from: this.opts.from == null ? undefined : String(this.opts.from),
      to: this.opts.to == null ? undefined : String(this.opts.to)
    }

    if (this.opts.map === false) {
      options.map = false
    } else {
      let map = this.mapOpts
      options.map = true
      if (typeof map.inline === 'boolean') options.mapInline = map.inline
      if (typeof map.sourcesContent === 'boolean') {
        options.mapSourcesContent = map.sourcesContent
      }
      if (typeof map.annotation !== 'undefined') {
        options.mapAnnotation =
          typeof map.annotation === 'string' ? map.annotation : map.annotation
      }
      if (typeof map.from === 'string') options.mapFrom = map.from
      if (map.absolute) options.mapAbsolute = true

      let prev = this.previousMapText(map.prev)
      if (prev) options.mapPrev = prev
    }

    let result = native.stringifyWithMap(this.root.toJSON(), options)
    return [result.css, result.map == null ? undefined : new MapText(result.map)]
  }

  // `map.prev` may be text, an object, a consumer or a generator. Anything the
  // caller did not supply falls back to a map the input itself carried — that
  // is how a `.scss` map reaches the output when only `from` was given.
  previousMapText(prev) {
    if (prev) {
      if (typeof prev === 'string') return prev
      if (typeof prev === 'function') {
        let value = prev(this.opts.from)
        return value ? this.previousMapText(value) : undefined
      }
      if (typeof prev.toString === 'function' && !(prev instanceof Object.getPrototypeOf(Object))) {
        let text = prev.toString()
        if (text !== '[object Object]') return text
      }
      return JSON.stringify(prev)
    }

    // A single input is the normal case. With several inputs — a plugin having
    // inlined `@import`s — only the first one's map is chained; the rest would
    // need per-input maps, which the AST does not carry.
    let inputs = new Set()
    this.root.walk(node => {
      if (node.source) inputs.add(node.source.input)
    })
    if (this.root.source) inputs.add(this.root.source.input)
    for (let input of inputs) {
      if (input.map && input.map.text) return input.map.text
    }
    return undefined
  }
}

// `result.map` is expected to have `toString()` and `toJSON()`, as
// `source-map-js`'s generator does.
class MapText {
  constructor(text) {
    this.text = text
  }

  toJSON() {
    return JSON.parse(this.text)
  }

  toString() {
    return this.text
  }
}

module.exports = MapGenerator
MapGenerator.default = MapGenerator
