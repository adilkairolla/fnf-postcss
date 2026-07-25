// Part of the PostCSS-compatible JS layer of `fnf-postcss`, taken from PostCSS
// (https://github.com/postcss/postcss, MIT, Copyright 2013 Andrey Sitnik) so
// that plugins see exactly the object model they were written against. Parsing,
// stringification, source maps and tokenizing are handled by the Rust core; see
// lib/parse.js, lib/map-generator.js and lib/source-map.js.

/* eslint-disable no-console */
'use strict'

let printed = {}

module.exports = function warnOnce(message) {
  if (printed[message]) return
  printed[message] = true

  if (typeof console !== 'undefined' && console.warn) {
    console.warn(message)
  }
}
