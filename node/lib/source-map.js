'use strict'

// The slice of `source-map-js` that PostCSS's JS layer uses, backed by the Rust
// core's own source-map code — the same code the crate's map generation is
// verified against, so there is one implementation rather than two.
//
// Only what `input.js` and `previous-map.js` actually call is here:
// `originalPositionFor`, `sourceContentFor`, the `file`/`sourceRoot`/`sources`/
// `sourcesContent` fields, and enough of `SourceMapGenerator` to satisfy the
// `instanceof` checks and `fromSourceMap(…).toString()`.

let native = require('../index.js')

class SourceMapConsumer {
  constructor(map) {
    let text = typeof map === 'string' ? map : JSON.stringify(map)
    this.native = new native.MapConsumer(text)
    this.file = this.native.file ?? undefined
    this.sourceRoot = this.native.sourceRoot ?? undefined
    this.sources = this.native.sources
    let content = this.native.sourcesContent
    // Rust hands back `null` for a source with no recorded text; the JS API
    // uses `null` there too, so only the absent case needs translating.
    this.sourcesContent = content == null ? undefined : content
  }

  destroy() {}

  originalPositionFor({ column, line }) {
    let found = this.native.originalPositionFor(line, column)
    return {
      column: found.column ?? null,
      line: found.line ?? null,
      name: found.name ?? null,
      source: found.source ?? null
    }
  }

  sourceContentFor(source) {
    return this.native.sourceContentFor(source) ?? null
  }
}

class SourceMapGenerator {
  static fromSourceMap(consumer) {
    let generator = new SourceMapGenerator()
    generator.text = consumer.native.toJsonString()
    return generator
  }

  constructor(text) {
    this.text = text
  }

  toString() {
    return this.text
  }
}

module.exports = { SourceMapConsumer, SourceMapGenerator }
