// Turns built addons into the per-platform npm packages that
// `optionalDependencies` points at, one directory per binary under `npm/`.
//
// npm installs only the package whose `os`/`cpu`/`libc` match the machine, so a
// user downloads one binary rather than eight.
//
//   node scripts/platform-packages.mjs <addon.node>...
//
// Each argument is named `fnf-postcss.<platform>.node`, which is what
// `scripts/build.mjs` produces and what `index.js` looks for.

import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const manifest = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'))

// What each platform suffix means to npm.
const PLATFORMS = {
  'darwin-arm64': { cpu: ['arm64'], os: ['darwin'] },
  'darwin-x64': { cpu: ['x64'], os: ['darwin'] },
  'linux-arm64-gnu': { cpu: ['arm64'], libc: ['glibc'], os: ['linux'] },
  'linux-arm64-musl': { cpu: ['arm64'], libc: ['musl'], os: ['linux'] },
  'linux-x64-gnu': { cpu: ['x64'], libc: ['glibc'], os: ['linux'] },
  'linux-x64-musl': { cpu: ['x64'], libc: ['musl'], os: ['linux'] },
  'win32-arm64-msvc': { cpu: ['arm64'], os: ['win32'] },
  'win32-x64-msvc': { cpu: ['x64'], os: ['win32'] }
}

const addons = process.argv.slice(2)
if (addons.length === 0) {
  console.error('usage: node scripts/platform-packages.mjs <addon.node>...')
  process.exit(2)
}

let written = []

for (const addon of addons) {
  const name = basename(addon)
  const match = /^fnf-postcss\.(.+)\.node$/.exec(name)
  if (!match) {
    console.error(`skipping ${name}: not named fnf-postcss.<platform>.node`)
    process.exitCode = 1
    continue
  }

  const platform = match[1]
  const traits = PLATFORMS[platform]
  if (!traits) {
    console.error(`skipping ${name}: unknown platform "${platform}"`)
    process.exitCode = 1
    continue
  }

  const dir = join(ROOT, 'npm', platform)
  mkdirSync(dir, { recursive: true })
  copyFileSync(addon, join(dir, name))

  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify(
      {
        name: `@fnf-postcss/${platform}`,
        version: manifest.version,
        description: `Native addon of fnf-postcss for ${platform}`,
        license: manifest.license,
        repository: manifest.repository,
        main: name,
        files: [name],
        ...traits,
        engines: manifest.engines
      },
      null,
      2
    ) + '\n'
  )

  writeFileSync(
    join(dir, 'README.md'),
    `# @fnf-postcss/${platform}\n\n` +
      `The native addon of [fnf-postcss](https://www.npmjs.com/package/fnf-postcss) ` +
      `for ${platform}. Installed automatically as an optional dependency; there is ` +
      `no reason to depend on it directly.\n`
  )

  written.push(platform)
}

// The main package's optionalDependencies must list exactly what exists, at the
// same version, or npm silently installs nothing and the loader throws.
const expected = Object.keys(manifest.optionalDependencies ?? {})
  .map(name => name.replace('@fnf-postcss/', ''))
  .sort()
if (written.sort().join(',') !== expected.join(',')) {
  console.error(
    `\nWARNING: built [${written.join(', ')}]\n` +
      `         but optionalDependencies expects [${expected.join(', ')}]`
  )
  process.exitCode = 1
}

console.log(`${written.length} platform packages in npm/: ${written.join(', ')}`)
