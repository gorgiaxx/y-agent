/// <reference types="node" />

import { readdirSync, readFileSync } from 'node:fs'
import { extname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'
import { createGenerator } from 'unocss'

import unoConfig from '../../uno.config'

/**
 * UnoCSS scans every source file for utility candidates, so plain English
 * words in comments and strings can generate global utilities. Some of
 * those names collide with the token classes that react-syntax-highlighter
 * (Prism/refractor) puts on code spans -- e.g. TOML table headers become
 * <span class="token table">, and a stray `.table{display:table}` utility
 * then forces `[table.header]` to break across three lines.
 *
 * These tests pin the two layers of protection:
 *   1. uno.config.ts must not emit a `.table` display utility.
 *   2. styles/index.css must keep Prism token spans inline, so any other
 *      stray utility (e.g. `.block`, legitimately used by Switch) cannot
 *      break highlighted code lines apart.
 */

const SCANNED_EXTENSIONS = ['.ts', '.tsx', '.js', '.jsx', '.html']

/** Concatenate every production source file UnoCSS would scan. */
function productionSources(): string {
  const root = fileURLToPath(new URL('../', import.meta.url))
  const chunks: string[] = []

  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name)

      if (entry.isDirectory()) {
        if (entry.name !== '__tests__') visit(path)
        continue
      }

      if (SCANNED_EXTENSIONS.includes(extname(entry.name))) {
        chunks.push(readFileSync(path, 'utf8'))
      }
    }
  }

  visit(root)
  return chunks.join('\n')
}

describe('UnoCSS / Prism token class collisions', () => {
  it('never generates a .table display utility from source scanning', async () => {
    const uno = await createGenerator(unoConfig)
    const { css } = await uno.generate(productionSources(), { preflights: false })

    expect(css).not.toMatch(/\.table\s*\{[^}]*display\s*:\s*table/)
  })

  it('keeps Prism token spans inline inside highlighted code', () => {
    const styles = readFileSync(new URL('../styles/index.css', import.meta.url), 'utf8')
    const guard = styles.match(/code\[class\*="language-"\]\s+\.token\s*\{([^}]*)\}/s)

    expect(guard, 'Missing Prism token display guard in styles/index.css').not.toBeNull()
    expect(guard?.[1]).toMatch(/display\s*:\s*inline\s*;/)
  })
})
