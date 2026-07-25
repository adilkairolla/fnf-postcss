'use strict'

// Stands in for `picocolors`, which the error formatting uses. Only the pieces
// PostCSS reaches for are here, with the same colour-support rules: honour
// FORCE_COLOR / NO_COLOR / TERM=dumb, otherwise colour only a TTY.

let env = process.env || {}
let argv = process.argv || []
let isTTY = process.stdout && process.stdout.isTTY

let isColorSupported =
  !('NO_COLOR' in env) &&
  !argv.includes('--no-color') &&
  (('FORCE_COLOR' in env) ||
    argv.includes('--color') ||
    (process.platform === 'win32' && !(env.TERM === 'dumb')) ||
    (Boolean(isTTY) && env.TERM !== 'dumb') ||
    'CI' in env)

function formatter(open, close) {
  return enabled =>
    enabled
      ? input => {
          let string = String(input)
          // Re-open the colour after any nested reset, as picocolors does, so
          // adjacent styles do not swallow each other.
          let index = string.indexOf(close, open.length)
          return index === -1
            ? open + string + close
            : open + string.replaceAll(close, close + open) + close
        }
      : String
}

const STYLES = {
  bold: ['[1m', '[22m'],
  cyan: ['[36m', '[39m'],
  gray: ['[90m', '[39m'],
  green: ['[32m', '[39m'],
  magenta: ['[35m', '[39m'],
  red: ['[31m', '[39m'],
  yellow: ['[33m', '[39m']
}

function createColors(enabled = isColorSupported) {
  let colors = { isColorSupported: enabled }
  for (let name in STYLES) {
    colors[name] = formatter(STYLES[name][0], STYLES[name][1])(enabled)
  }
  return colors
}

module.exports = { ...createColors(), createColors, isColorSupported }
