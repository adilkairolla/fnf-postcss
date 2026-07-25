'use strict'

// `terminal-highlight.js` tokenizes the broken CSS to colour the snippet in an
// error message. The Rust core has the tokenizer, so this adapts its output to
// the pull-style interface that file expects.

let native = require('../index.js')

module.exports = function tokenizer(input) {
  let tokens = native.tokenize(input.css)
  let position = 0

  return {
    endOfFile() {
      return position >= tokens.length
    },
    nextToken() {
      if (position >= tokens.length) return undefined
      return tokens[position++]
    },
    back(token) {
      if (token) position -= 1
    },
    position() {
      return position
    }
  }
}
