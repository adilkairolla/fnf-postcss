'use strict'

// Loads the native addon for the current platform.
//
// A local build (`npm run build`) drops the binary next to this file; a published
// install gets it from one of the `@fnf-postcss/*` optional dependencies, of
// which npm installs only the one matching the platform.

const { existsSync } = require('fs')
const { join } = require('path')

const { arch, platform } = process

function libc() {
  if (platform !== 'linux') return null
  // musl builds have no glibc report, which is how we tell the two apart.
  let report = process.report && process.report.getReport()
  let header = report && report.header
  if (header && header.glibcVersionRuntime) return 'gnu'
  return 'musl'
}

function candidates() {
  // On Linux the libc flavour is part of the name; everywhere else the two
  // spellings coincide, so drop the duplicate.
  let flavour = libc()
  let names = [[platform, arch, flavour].filter(Boolean).join('-')]
  let bare = `${platform}-${arch}`
  if (!names.includes(bare)) names.push(bare)
  return names
}

let loaded
let attempted = []

for (let target of candidates()) {
  let local = join(__dirname, `fnf-postcss.${target}.node`)
  if (existsSync(local)) {
    loaded = require(local)
    break
  }
  let pkg = `@fnf-postcss/${target}`
  attempted.push(pkg)
  try {
    loaded = require(pkg)
    break
  } catch (error) {
    if (error.code !== 'MODULE_NOT_FOUND') throw error
  }
}

if (!loaded) {
  throw new Error(
    `fnf-postcss has no prebuilt binary for ${platform}-${arch}.\n` +
      `Tried: ${attempted.join(', ')}.\n` +
      `Build one from source with: npm run build --prefix node_modules/fnf-postcss`
  )
}

module.exports = loaded
