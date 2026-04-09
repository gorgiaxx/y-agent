import { defineConfig } from 'vitepress';
import { withMermaid } from 'vitepress-plugin-mermaid';

const SITE_URL = 'https://y-agent.dev';

export default withMermaid(defineConfig({
  title: 'y-agent',
  description:
    'A modular, extensible AI agent framework written in Rust. Async-first, model-agnostic, full observability, WAL-based recoverability, self-evolving skills.',

  base: '/',
  lastUpdated: true,
  cleanUrls: true,

  sitemap: {
    hostname: SITE_URL,
  },

  vite: {
    build: {
      target: 'esnext',
    },
    optimizeDeps: {
      esbuildOptions: {
        target: 'esnext',
      },
    },
  },

  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
    ['meta', { name: 'theme-color', content: '#e8590c' }],
    ['meta', { name: 'author', content: 'y-agent Team' }],
    [
      'meta',
      {
        name: 'keywords',
        content:
          'y-agent, AI agent framework, Rust, LLM, multi-agent, DAG workflow, tool calling, MCP, knowledge base, RAG, self-evolving skills, async-first, model-agnostic',
      },
    ],
    ['meta', { name: 'robots', content: 'index, follow' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'y-agent' }],
    [
      'meta',
      {
        property: 'og:title',
        content: 'y-agent -- Modular AI Agent Framework in Rust',
      },
    ],
    [
      'meta',
      {
        property: 'og:description',
        content:
          'Async-first, model-agnostic AI agent framework with full observability, WAL-based recoverability, and self-evolving skills.',
      },
    ],
    ['meta', { property: 'og:url', content: SITE_URL }],
    ['meta', { property: 'og:locale', content: 'en' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    [
      'meta',
      {
        name: 'twitter:title',
        content: 'y-agent -- Modular AI Agent Framework in Rust',
      },
    ],
    [
      'meta',
      {
        name: 'twitter:description',
        content:
          'Async-first, model-agnostic AI agent framework with full observability and self-evolving skills.',
      },
    ],
  ],

  locales: {
    root: {
      label: 'English',
      lang: 'en',
      title: 'y-agent',
      description:
        'A modular, extensible AI agent framework written in Rust.',
      themeConfig: {
        nav: [
          { text: 'Home', link: '/' },
          { text: 'Download', link: '/download' },
          { text: 'Docs', link: '/guide/getting-started' },
          { text: 'Architecture', link: '/architecture/' },
        ],
        sidebar: {
          '/guide/': [
            {
              text: 'Guide',
              items: [
                {
                  text: 'Getting Started',
                  link: '/guide/getting-started',
                },
                {
                  text: 'Configuration',
                  link: '/guide/configuration',
                },
                {
                  text: 'GUI Desktop App',
                  link: '/guide/gui-desktop',
                },
                {
                  text: 'Knowledge Base',
                  link: '/guide/knowledge-base',
                },
                {
                  text: 'Bot Adapters',
                  link: '/guide/bot-adapters',
                },
                { text: 'Web API', link: '/guide/web-api' },
              ],
            },
          ],
          '/architecture/': [
            {
              text: 'Architecture',
              items: [
                { text: 'Overview', link: '/architecture/' },
                { text: 'Crate Map', link: '/architecture/crate-map' },
              ],
            },
          ],
          '/deployment/': [
            {
              text: 'Deployment',
              items: [
                { text: 'Overview', link: '/deployment/' },
              ],
            },
          ],
        },
      },
    },
  },

  themeConfig: {
    logo: '/logo.svg',
    socialLinks: [
      { icon: 'github', link: 'https://github.com/gorgias/y-agent' },
    ],
    search: {
      provider: 'local',
    },
    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright 2024-present y-agent Team',
    },
  },
}));
