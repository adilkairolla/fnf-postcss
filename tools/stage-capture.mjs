// Runs an app's real PostCSS pipeline one plugin at a time and dumps the CSS
// after each stage, with the map each stage produced. Those dumps are the actual
// intermediate states the app's plugins hand to each other — the inputs worth
// feeding through this crate.
//
//   APP_DIR=/path/to/app node stage-capture.mjs <outdir> <entry.css>...

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, join, relative, resolve } from 'node:path'
import { createRequire } from 'node:module'

// PostCSS is resolved from the app under test (`APP_DIR`), so the comparison
// runs against the exact version that app's build uses, with its plugins
// resolvable. Falls back to this repo's tools/node_modules copy.
const require_ = createRequire(
  process.env.APP_DIR ? join(process.env.APP_DIR, 'package.json') : import.meta.url
)
const postcss = require_('postcss')
const postcssrc = require_('postcss-load-config')

const APP = process.env.APP_DIR ?? process.cwd()
const [outdir, ...entries] = process.argv.slice(2)
if (!outdir || entries.length === 0) {
  console.error('usage: node stage-capture.mjs <outdir> <entry.css>...')
  process.exit(2)
}
mkdirSync(outdir, { recursive: true })

// Production is where the interesting plugins live (full preset-env, the
// cascade-layer polyfill and the downlevel passes).
process.env.NODE_ENV = 'production'
process.env.POSTCSS_FULL = '1'

const { plugins } = await postcssrc({ env: 'production' }, APP)
console.log(`pipeline: ${plugins.map(p => p.postcssPlugin ?? '(anon)').join(' → ')}`)

const manifest = []

for (const entry of entries) {
  const from = resolve(entry)
  const label = relative(APP, from).replace(/[/\\]/g, '__')
  let css = readFileSync(from, 'utf8')
  let prev

  // Stage 0 is the untransformed source as PostCSS itself sees it.
  const dump = (stage, name, text, map) => {
    const file = join(outdir, `${label}.${String(stage).padStart(2, '0')}.${name}.css`)
    writeFileSync(file, text)
    if (map) writeFileSync(`${file}.map`, map)
    manifest.push({ bytes: Buffer.byteLength(text), entry: relative(APP, from), file, stage: name })
    return file
  }
  dump(0, 'source', css)

  for (const [index, plugin] of plugins.entries()) {
    const name = (plugin.postcssPlugin ?? `plugin${index}`).replace(/[^\w.-]/g, '_')
    let result
    try {
      result = await postcss([plugin]).process(css, {
        from,
        // The same map shape Vite hands PostCSS: a real chained map, not inline.
        map: { annotation: false, inline: false, prev, sourcesContent: true },
        to: from
      })
    } catch (e) {
      console.log(`  ${relative(APP, from)}: ${name} threw ${e.name}: ${e.message.split('\n')[0]}`)
      break
    }
    css = result.css
    prev = result.map ? result.map.toString() : prev
    dump(index + 1, name, css, prev)
  }
  console.log(`  ${relative(APP, from)} → ${(Buffer.byteLength(css) / 1024).toFixed(0)} KiB`)
}

writeFileSync(join(outdir, 'manifest.json'), JSON.stringify(manifest, null, 2))
console.log(`\n${manifest.length} stage dumps in ${outdir}`)
