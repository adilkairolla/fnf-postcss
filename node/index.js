'use strict'

// Loads the native addon for this machine.
//
// The published package bundles one prebuilt addon (darwin-arm64). Anywhere
// else, `npm run build` compiles one with cargo and writes it next to this file
// under the same naming scheme, so it is found the same way.

const { existsSync } = require('fs')
const { join } = require('path')

const { arch, platform } = process

// The suffix must match what scripts/build.mjs writes.
function hostTarget() {
  if (platform === 'linux') {
    // musl builds have no glibc report, which is how the two are told apart.
    const report = process.report && process.report.getReport()
    const flavour =
      report && report.header && report.header.glibcVersionRuntime ? 'gnu' : 'musl'
    return `linux-${arch}-${flavour}`
  }
  if (platform === 'win32') return `win32-${arch}-msvc`
  return `${platform}-${arch}`
}

const target = hostTarget()
const addon = join(__dirname, `fnf-postcss.${target}.node`)

if (!existsSync(addon)) {
  throw new Error(
    `fnf-postcss has no addon for ${target}.\n` +
      `The published package bundles darwin-arm64 only; build one for this ` +
      `machine (needs a Rust toolchain — https://rustup.rs):\n` +
      `  npm run build --prefix node_modules/fnf-postcss`
  )
}

module.exports = require(addon)
