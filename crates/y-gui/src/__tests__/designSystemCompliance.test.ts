/// <reference types="node" />

import { readFileSync, readdirSync } from 'node:fs'
import { extname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

function readSource(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), 'utf8')
}

function cssRule(source: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = source.match(new RegExp(`${escapedSelector}\\s*\\{([^}]*)\\}`, 's'))

  expect(match, `Missing CSS rule for ${selector}`).not.toBeNull()
  return match?.[1] ?? ''
}

function productionStyleSources(): Array<[string, string]> {
  const root = fileURLToPath(new URL('../', import.meta.url))
  const files: Array<[string, string]> = []

  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name)

      if (entry.isDirectory()) {
        if (entry.name !== '__tests__') visit(path)
        continue
      }

      if (['.css', '.ts', '.tsx'].includes(extname(entry.name))) {
        files.push([path, readFileSync(path, 'utf8')])
      }
    }
  }

  visit(root)
  return files
}

describe('DESIGN.md visual contracts', () => {
  it('uses the 4px and 8px radius system without a 12px application token', () => {
    const styles = readSource('../styles/index.css')

    expect(styles).toMatch(/--radius-sm:\s*4px;/)
    expect(styles).toMatch(/--radius-md:\s*8px;/)
    expect(styles).not.toMatch(/--radius-lg:\s*12px;/)
  })

  it('keeps normal body text at zero letter spacing', () => {
    const styles = readSource('../styles/index.css')

    expect(cssRule(styles, 'body')).toMatch(/letter-spacing:\s*0;/)
    expect(styles).not.toMatch(/letter-spacing:\s*-[\d.]+(?:em|px|rem)/)
  })

  it('gives shared buttons and fields the 4px control radius', () => {
    const button = readSource('../components/ui/Button.tsx')
    const input = readSource('../components/ui/Input.tsx')
    const select = readSource('../components/ui/Select.tsx')
    const tabs = readSource('../components/ui/Tabs.tsx')

    expect(button).toContain("'rounded-sm'")
    expect(button).not.toContain("'rounded-md'")
    expect(input.match(/rounded-\[var\(--radius-sm\)\]/g)).toHaveLength(2)
    expect(select).toMatch(/SelectPrimitive\.Trigger[\s\S]*?rounded-\[var\(--radius-sm\)\]/)
    expect(tabs).not.toContain("'rounded-[6px]'")
    expect(tabs).toMatch(/TabsPrimitive\.Trigger[\s\S]*?'rounded-\[var\(--radius-sm\)\]'/)
  })

  it('uses 8px for overlays and 4px for shell controls', () => {
    const dialog = readSource('../components/ui/Dialog.tsx')
    const windowControls = readSource('../components/ui/WindowControls.css')
    const app = readSource('../App.css')
    const nav = readSource('../components/common/NavSidebar/NavSidebar.css')
    const inputArea = readSource('../components/chat-panel/input-area/InputArea.css')

    expect(dialog).toContain("'rounded-[var(--radius-md)]'")
    expect(dialog).not.toContain('var(--radius-lg)')
    expect(cssRule(windowControls, '.window-control-btn')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(app, '.btn-header')).toMatch(/border-radius:\s*var\(--radius-sm\);/)
    expect(cssRule(nav, '.nav-item')).toMatch(/border-radius:\s*var\(--radius-sm\);/)
    expect(cssRule(inputArea, '.input-container')).toMatch(
      /border-radius:\s*var\(--radius-md\);/,
    )
    expect(cssRule(inputArea, '.btn-send')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
  })

  it('does not retain oversized legacy corners or decorative gradients', () => {
    const violations = productionStyleSources().flatMap(([path, source]) => {
      const reasons = []
      if (/radius-lg|rounded-lg/.test(source)) reasons.push('legacy large radius')
      if (/border-radius:\s*10px/.test(source)) reasons.push('10px radius')
      if (/[a-z-]+-gradient\(/.test(source)) reasons.push('gradient')
      return reasons.map((reason) => `${path}: ${reason}`)
    })

    expect(violations).toEqual([])
  })

  it('uses semantic theme tokens for shared component depth and state colors', () => {
    const button = readSource('../components/ui/Button.tsx')
    const badge = readSource('../components/ui/Badge.tsx')
    const dialog = readSource('../components/ui/Dialog.tsx')
    const popover = readSource('../components/ui/Popover.tsx')
    const providerIconPicker = readSource('../components/common/ProviderIconPicker.tsx')
    const scrollArea = readSource('../components/ui/ScrollArea.tsx')
    const select = readSource('../components/ui/Select.tsx')
    const toast = readSource('../components/ui/Toast.tsx')
    const tooltip = readSource('../components/ui/Tooltip.tsx')

    expect(button).not.toMatch(/text-\[#[0-9a-fA-F]+\]/)
    expect(badge).not.toMatch(/rgba?\(/)
    expect(dialog).toContain("'shadow-lg'")
    expect(popover).toContain("'shadow-md'")
    expect(select).toContain("'shadow-md'")
    expect(toast).not.toMatch(/rgba?\(/)
    expect(tooltip).toContain("'shadow-md'")
    expect(providerIconPicker).not.toMatch(/border-\[rgba?\(/)
    expect(scrollArea).toContain('bg-[var(--scrollbar-thumb)]')
    expect(scrollArea).toContain('hover:bg-[var(--scrollbar-thumb-hover)]')
  })

  it('keeps specialized interactive rows and controls on the 4px radius', () => {
    const automation = readSource('../components/automation/AutomationPanel.css')
    const backgroundTasks = readSource(
      '../components/background-tasks/BackgroundTasksPanel.css',
    )
    const info = readSource('../components/observation/InfoPanel.css')
    const resume = readSource('../components/chat-panel/ResumeSessionDialog.css')
    const wizard = readSource('../components/wizard/SetupWizard.css')

    expect(cssRule(automation, '.automation-sidebar-item')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(automation, '.automation-btn')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(automation, '.automation-editor-select')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(backgroundTasks, '.background-tasks-sidebar-item')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(info, '.info-subagent-item')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(resume, '.resume-session-item')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
    expect(cssRule(wizard, '.wizard-api-type-btn')).toMatch(
      /border-radius:\s*var\(--radius-sm\);/,
    )
  })

  it('keeps the setup wizard inside narrow viewports', () => {
    const wizard = readSource('../components/wizard/SetupWizard.css')

    expect(cssRule(wizard, '.wizard-api-types')).toMatch(
      /grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\);/,
    )
    expect(cssRule(wizard, '.wizard-api-type-btn')).toMatch(/min-width:\s*0;/)
    expect(wizard).toMatch(
      /@media\s*\(max-width:\s*640px\)[\s\S]*?\.wizard-api-types\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/,
    )
  })
})
