// Builds the native addon and puts it where `index.js` looks for it.
//
//   node scripts/build.mjs [--target <rust-triple>] [--debug]
//
// This drives cargo directly rather than going through `napi build`: the only
// thing that step adds is copying and renaming the artifact, and the CLI's idea
// of what cargo names a `cdylib` differs between versions and platforms.

import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const NODE_DIR = resolve(HERE, '..')
const REPO = resolve(NODE_DIR, '..')

const args = process.argv.slice(2)
const target = args.includes('--target') ? args[args.indexOf('--target') + 1] : undefined
const profile = args.includes('--debug') ? 'debug' : 'release'
// `--if-missing` builds only when there is no addon yet, and gives up quietly
// rather than failing when there is one but no toolchain to rebuild it with.
const ifMissing = args.includes('--if-missing')

// rustup installed with --no-modify-path leaves cargo out of a login shell's
// PATH, so `npm run …` cannot see it. Look where rustup puts it before giving up.
function findCargo() {
  if (process.env.CARGO) return process.env.CARGO
  const exe = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
  const installed = join(process.env.CARGO_HOME || join(homedir(), '.cargo'), 'bin', exe)
  if (existsSync(installed)) return installed
  // Fall back to the name and let the OS search PATH.
  return exe
}

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

const destination = join(NODE_DIR, `fnf-postcss.${suffix}.node`)

if (ifMissing && existsSync(destination)) {
  console.log(`${destination} is already built`)
  process.exit(0)
}

const cargo = ['build', '-p', 'fnf-postcss-node']
if (profile === 'release') cargo.push('--release')
if (target) cargo.push('--target', target)

const cargoBin = findCargo()
console.log(`${cargoBin} ${cargo.join(' ')}`)
try {
  execFileSync(cargoBin, cargo, { cwd: REPO, stdio: 'inherit' })
} catch (error) {
  if (error.code === 'ENOENT') {
    console.error(
      `\ncargo not found. Install a Rust toolchain from https://rustup.rs, or ` +
        `set CARGO to its path.\n` +
        `(rustup installed with --no-modify-path leaves it out of PATH; ` +
        `\`. "$HOME/.cargo/env"\` fixes that.)`
    )
    process.exit(1)
  }
  throw error
}

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

copyFileSync(built, destination)
console.log(`${built} → ${destination}`)
