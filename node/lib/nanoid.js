'use strict'

// Stands in for `nanoid/non-secure`. The id only has to make the placeholder
// `<input css …>` name unique within a process, so `Math.random` is plenty —
// this is exactly what the non-secure entry point of nanoid is for.

const ALPHABET =
  'useandom-26T198340PX75pxJACKVERYMINDBUSHWOLFGQZbfghjklqvwyzrict'

function nanoid(size = 21) {
  let id = ''
  for (let index = 0; index < size; index++) {
    id += ALPHABET[(Math.random() * ALPHABET.length) | 0]
  }
  return id
}

module.exports = { nanoid }
