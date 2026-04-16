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
        contrast: 'var(--accent-contrast)',
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
    // Surface card
    'surface-card': 'bg-surface-primary border border-solid border-border rounded-lg shadow-lg',
  },
})
