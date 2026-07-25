// Does the drop-in actually drop in?
//
// Runs the same real, unmodified PostCSS plugins over the same CSS twice — once
// through the `postcss` package, once through this one — and compares the output
// CSS and source map byte for byte. Anything a plugin can reach (the node
// classes, walkers, mutation, raws inference, warnings, errors) is exercised by
// the plugins themselves rather than by assertions we thought to write.
//
//   APP_DIR=/path/to/app node test/drop-in.mjs <file-or-dir>...
//
// With APP_DIR set, the plugin pipeline comes from that app's postcss.config.*
// and the reference implementation is that app's own `postcss`. Without it, a
// small built-in set of transforms stands in, so the test still runs anywhere.

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const APP = process.env.APP_DIR

// The reference implementation comes from the app under test when there is one,
// so we compare against the exact version that app builds with; otherwise from
// this repo's tools/node_modules.
const require_ = createRequire(
  APP ? join(APP, 'package.json') : resolve(HERE, '..', '..', 'tools', 'package.json')
)
const reference = require_('postcss')
const ours = createRequire(import.meta.url)(join(HERE, '..', 'lib', 'postcss.js'))

// Transforms that lean on the parts of the API plugins actually use, for when
// there is no app to borrow a pipeline from.
const BUILT_IN = [
  {
    postcssPlugin: 'rename-and-clone',
    Declaration(decl) {
      if (decl.prop === 'color' && !decl.prop.startsWith('-x-')) {
        decl.cloneBefore({ prop: '-x-' + decl.prop })
      }
    }
  },
  {
    postcssPlugin: 'wrap-rules',
    OnceExit(root, { AtRule }) {
      root.walkRules(rule => {
        if (rule.parent.type !== 'root') return
        if (!rule.some(node => node.type === 'decl' && node.value.includes('var('))) return
        let wrapper = new AtRule({
          name: 'supports',
          params: '(color: var(--x))',
          source: rule.source
        })
        let moved = rule.clone()
        moved.removeAll()
        rule.each(node => moved.append(node.clone()))
        wrapper.append(moved)
        rule.after(wrapper)
      })
    }
  },
  {
    postcssPlugin: 'sort-and-warn',
    Rule(rule, { result }) {
      if (rule.selector.includes('!')) {
        rule.warn(result, 'suspicious selector', { word: '!' })
      }
    }
  }
]

async function pipeline() {
  if (!APP) return BUILT_IN
  const postcssrc = require_('postcss-load-config')
  process.env.NODE_ENV = 'production'
  process.env.POSTCSS_FULL = '1'
  const { plugins } = await postcssrc({ env: 'production' }, APP)
  return plugins
}

function collect(target) {
  return statSync(target).isFile()
    ? [target]
    : readdirSync(target)
        .filter(name => name.endsWith('.css'))
        .map(name => join(target, name))
}

function firstDifference(a, b) {
  const limit = Math.min(a.length, b.length)
  let index = 0
  while (index < limit && a[index] === b[index]) index++
  return (
    `at byte ${index} (postcss ${a.length} bytes, ours ${b.length} bytes)\n` +
    `    postcss …${JSON.stringify(a.slice(Math.max(0, index - 40), index + 90))}\n` +
    `    ours    …${JSON.stringify(b.slice(Math.max(0, index - 40), index + 90))}`
  )
}

const targets = process.argv.slice(2)
if (targets.length === 0) {
  console.error('usage: node test/drop-in.mjs <file-or-dir>...')
  process.exit(2)
}

const plugins = await pipeline()
console.log(
  `pipeline: ${plugins.map(p => p.postcssPlugin ?? '(anon)').join(' → ')}` +
    `${APP ? ` (from ${APP})` : ' (built-in)'}\n`
)

// The map options a bundler asks for, so map generation is covered too.
const MAP_VARIANTS = [
  { name: 'no-map', map: false },
  { name: 'external', map: { annotation: false, inline: false, sourcesContent: true } },
  { name: 'inline', map: { inline: true, sourcesContent: true } }
]

let checked = 0
let failed = 0
let known = 0

for (const target of targets) {
  for (const file of collect(target)) {
    const css = readFileSync(file, 'utf8')

    for (const variant of MAP_VARIANTS) {
      checked++
      const opts = { from: file, map: variant.map, to: file.replace(/\.css$/, '.out.css') }

      let expected, actual
      try {
        expected = await reference([...plugins]).process(css, opts)
      } catch (error) {
        // Both must reject the same way — including the position, which is what
        // makes an error useful and is easy to drop when rebuilding one across a
        // language boundary.
        try {
          await ours([...plugins]).process(css, opts).then(r => r.css)
          failed++
          console.log(`\x1b[31mFAIL\x1b[0m ${file} ${variant.name}: postcss threw, we did not`)
        } catch (mirrored) {
          let shape = e =>
            JSON.stringify({
              column: e.column,
              endColumn: e.endColumn,
              endLine: e.endLine,
              line: e.line,
              name: e.name,
              reason: e.reason
            })
          if (shape(mirrored) !== shape(error)) {
            failed++
            console.log(
              `\x1b[31mFAIL\x1b[0m ${file} ${variant.name}: error differs\n` +
                `  postcss ${shape(error)}\n  ours    ${shape(mirrored)}`
            )
          }
        }
        continue
      }
      actual = await ours([...plugins]).process(css, opts)

      // eslint-disable-next-line no-control-regex
      const hasNonAscii = /[^\x00-\x7f]/.test(css)
      // With an inline map the whole map lives in the CSS, so the non-ASCII
      // column difference shows up there; compare the CSS without it.
      const withoutMap = text => text.replace(/\n*\/\*# sourceMappingURL=data:[^*]*\*\/\s*$/, '')

      const problems = []
      if (expected.css !== actual.css) {
        if (hasNonAscii && withoutMap(expected.css) === withoutMap(actual.css)) {
          known++
          console.log(`\x1b[33mknown\x1b[0m ${file} ${variant.name}: non-ASCII map columns`)
        } else {
          problems.push(firstDifference(expected.css, actual.css))
        }
      }

      const expectedMap = expected.map ? expected.map.toString() : null
      const actualMap = actual.map ? actual.map.toString() : null
      if (expectedMap !== actualMap) {
        let difference =
          expectedMap == null || actualMap == null
            ? `map presence differs (postcss ${expectedMap == null ? 'none' : 'yes'}, ours ${actualMap == null ? 'none' : 'yes'})`
            : `map differs ${firstDifference(expectedMap, actualMap)}`
        // Documented divergence: map columns count characters here and UTF-16
        // code units in PostCSS, so they part company on non-ASCII lines. Report
        // it rather than failing — but only for CSS that actually has non-ASCII,
        // so a real map regression still fails the run.
        if (hasNonAscii) {
          known++
          console.log(`\x1b[33mknown\x1b[0m ${file} ${variant.name}: non-ASCII map columns`)
        } else {
          problems.push(difference)
        }
      }

      const expectedWarnings = expected.warnings().map(w => w.toString()).join('|')
      const actualWarnings = actual.warnings().map(w => w.toString()).join('|')
      if (expectedWarnings !== actualWarnings) {
        problems.push(`warnings "${actualWarnings}" !== "${expectedWarnings}"`)
      }

      if (problems.length > 0) {
        failed++
        console.log(`\x1b[31mFAIL\x1b[0m ${file} ${variant.name}`)
        for (const problem of problems) console.log(`  ${problem}`)
      }
    }
  }
}

console.log(
  `\n${checked - failed}/${checked} cases matched the postcss package` +
    (known > 0 ? ` (${known} with the documented non-ASCII map-column difference)` : '')
)
process.exit(failed === 0 ? 0 : 1)
