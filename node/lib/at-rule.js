'use strict'

// Part of the PostCSS-compatible JS layer of `fnf-postcss`, taken from PostCSS
// (https://github.com/postcss/postcss, MIT, Copyright 2013 Andrey Sitnik) so
// that plugins see exactly the object model they were written against. Parsing,
// stringification, source maps and tokenizing are handled by the Rust core; see
// lib/parse.js, lib/map-generator.js and lib/source-map.js.

let Container = require('./container')

class AtRule extends Container {
  constructor(defaults) {
    super(defaults)
    this.type = 'atrule'
  }

  append(...children) {
    if (!this.proxyOf.nodes) this.nodes = []
    return super.append(...children)
  }

  prepend(...children) {
    if (!this.proxyOf.nodes) this.nodes = []
    return super.prepend(...children)
  }
}

module.exports = AtRule
AtRule.default = AtRule

Container.registerAtRule(AtRule)
