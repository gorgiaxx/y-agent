import { defineConfig, presetUno } from 'unocss'
import transformerVariantGroup from '@unocss/transformer-variant-group'

export default defineConfig({
  presets: [
    presetUno(),
  ],
  transformers: [
    transformerVariantGroup(),
  ],
  theme: {
    colors: {
      surface: {
        primary: 'var(--surface-primary)',
        secondary: 'var(--surface-secondary)',
        tertiary: 'var(--surface-tertiary)',
        hover: 'var(--surface-hover)',
        code: 'var(--surface-code)',
      },
      text: {
        primary: 'var(--text-primary)',
        secondary: 'var(--text-secondary)',
        muted: 'var(--text-muted)',
      },
      accent: {
        DEFAULT: 'var(--accent)',
        hover: 'var(--accent-hover)',
        subtle: 'var(--accent-subtle)',
        glow: 'var(--accent-glow)',
      },
      success: 'var(--success)',
      error: {
        DEFAULT: 'var(--error)',
        subtle: 'var(--error-subtle)',
      },
      warning: 'var(--warning)',
      border: 'var(--border)',
    },
    borderRadius: {
      sm: 'var(--radius-sm)',
      md: 'var(--radius-md)',
      lg: 'var(--radius-lg)',
    },
    boxShadow: {
      sm: 'var(--shadow-sm)',
      md: 'var(--shadow-md)',
      lg: 'var(--shadow-lg)',
    },
  },
  shortcuts: {
    // Button variants
    'btn-base': 'inline-flex items-center justify-center font-medium cursor-pointer transition-all duration-150 font-sans outline-none border-none',
    'btn-primary': 'btn-base bg-accent text-[#0f0f0f] rounded-md px-4 py-1.5 text-xs hover:op-85',
    'btn-ghost': 'btn-base bg-transparent text-text-secondary rounded-md px-3 py-1.5 text-xs border border-solid border-border hover:(bg-surface-hover text-text-primary)',
    'btn-danger': 'btn-base bg-error text-white rounded-md px-4 py-1.5 text-xs hover:op-85',
    'btn-icon': 'btn-base bg-transparent text-text-muted w-7 h-7 rounded-md border border-solid border-transparent hover:(text-text-primary border-border bg-surface-hover)',
    // Input
    'input-base': 'w-full px-2 py-1.5 text-xs font-sans border border-solid border-border rounded-sm bg-surface-secondary text-text-primary outline-none transition-colors duration-150 focus:border-[rgba(255,255,255,0.15)]',
    // Surface card
    'surface-card': 'bg-surface-primary border border-solid border-border rounded-lg shadow-lg',
  },
})
