'use strict'

// Part of the PostCSS-compatible JS layer of `fnf-postcss`, taken from PostCSS
// (https://github.com/postcss/postcss, MIT, Copyright 2013 Andrey Sitnik) so
// that plugins see exactly the object model they were written against. Parsing,
// stringification, source maps and tokenizing are handled by the Rust core; see
// lib/parse.js, lib/map-generator.js and lib/source-map.js.

let Container = require('./container')
let list = require('./list')

class Rule extends Container {
  get selectors() {
    return list.comma(this.selector)
  }

  set selectors(values) {
    let match = this.selector ? this.selector.match(/,\s*/) : null
    let sep = match ? match[0] : ',' + this.raw('between', 'beforeOpen')
    this.selector = values.join(sep)
  }

  constructor(defaults) {
    super(defaults)
    this.type = 'rule'
    if (!this.nodes) this.nodes = []
  }
}

module.exports = Rule
Rule.default = Rule

Container.registerRule(Rule)
