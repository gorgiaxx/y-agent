import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import UnoCSS from 'unocss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    UnoCSS(),
    react(),
  ],
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
