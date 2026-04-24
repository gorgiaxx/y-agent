import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import UnoCSS from 'unocss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    UnoCSS(),
    react(),
  ],
  test: {
    setupFiles: './src/__tests__/setup.ts',
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          monaco: ['monaco-editor', 'react-monaco-editor'],
          mermaid: ['mermaid'],
        },
      },
    },
  },
})
