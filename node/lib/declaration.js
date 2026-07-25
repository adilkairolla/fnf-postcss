'use strict'

// Part of the PostCSS-compatible JS layer of `fnf-postcss`, taken from PostCSS
// (https://github.com/postcss/postcss, MIT, Copyright 2013 Andrey Sitnik) so
// that plugins see exactly the object model they were written against. Parsing,
// stringification, source maps and tokenizing are handled by the Rust core; see
// lib/parse.js, lib/map-generator.js and lib/source-map.js.

let Node = require('./node')

class Declaration extends Node {
  get variable() {
    return this.prop.startsWith('--') || this.prop[0] === '$'
  }

  constructor(defaults) {
    if (
      defaults &&
      typeof defaults.value !== 'undefined' &&
      typeof defaults.value !== 'string'
    ) {
      defaults = { ...defaults, value: String(defaults.value) }
    }
    super(defaults)
    this.type = 'decl'
  }
}

module.exports = Declaration
Declaration.default = Declaration
