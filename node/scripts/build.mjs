// Builds the native addon and puts it where `index.js` looks for it.
//
//   node scripts/build.mjs [--target <rust-triple>] [--debug]
//
// This drives cargo directly rather than going through `napi build`: the only
// thing that step adds is copying and renaming the artifact, and the CLI's idea
// of what cargo names a `cdylib` differs between versions and platforms.

import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const NODE_DIR = resolve(HERE, '..')
const REPO = resolve(NODE_DIR, '..')

const args = process.argv.slice(2)
const target = args.includes('--target') ? args[args.indexOf('--target') + 1] : undefined
const profile = args.includes('--debug') ? 'debug' : 'release'

// Rust triple → the platform suffix `index.js` and the platform packages use.
const SUFFIXES = {
  'aarch64-apple-darwin': 'darwin-arm64',
  'aarch64-pc-windows-msvc': 'win32-arm64-msvc',
  'aarch64-unknown-linux-gnu': 'linux-arm64-gnu',
  'aarch64-unknown-linux-musl': 'linux-arm64-musl',
  'x86_64-apple-darwin': 'darwin-x64',
  'x86_64-pc-windows-msvc': 'win32-x64-msvc',
  'x86_64-unknown-linux-gnu': 'linux-x64-gnu',
  'x86_64-unknown-linux-musl': 'linux-x64-musl'
}

function hostSuffix() {
  const { arch, platform } = process
  if (platform === 'linux') {
    let report = process.report && process.report.getReport()
    let flavour = report?.header?.glibcVersionRuntime ? 'gnu' : 'musl'
    return `linux-${arch}-${flavour}`
  }
  if (platform === 'win32') return `win32-${arch}-msvc`
  return `${platform}-${arch}`
}

const suffix = target ? SUFFIXES[target] : hostSuffix()
if (!suffix) {
  console.error(`unknown target "${target}"; expected one of:\n  ${Object.keys(SUFFIXES).join('\n  ')}`)
  process.exit(2)
}

const cargo = ['build', '-p', 'fnf-postcss-node']
if (profile === 'release') cargo.push('--release')
if (target) cargo.push('--target', target)

console.log(`cargo ${cargo.join(' ')}`)
execFileSync('cargo', cargo, { cwd: REPO, stdio: 'inherit' })

// cargo replaces hyphens with underscores in the artifact name, and each
// platform has its own prefix and extension.
const outDir = join(REPO, 'target', ...(target ? [target] : []), profile)
const artifacts = [
  'libfnf_postcss_node.dylib',
  'libfnf_postcss_node.so',
  'fnf_postcss_node.dll'
].map(name => join(outDir, name))

const built = artifacts.find(path => existsSync(path))
if (!built) {
  console.error(`no addon found in ${outDir}; looked for:\n  ${artifacts.join('\n  ')}`)
  process.exit(1)
}

const destination = join(NODE_DIR, `fnf-postcss.${suffix}.node`)
copyFileSync(built, destination)
console.log(`${built} → ${destination}`)
