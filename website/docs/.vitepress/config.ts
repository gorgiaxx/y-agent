import { defineConfig } from 'vitepress';
import { withMermaid } from 'vitepress-plugin-mermaid';

const SITE_URL = 'https://ai.jiahao.li';

export default withMermaid(defineConfig({
  title: 'y-agent',
  description:
    'A Rust-first, model-agnostic Agent Harness with plan and loop execution, self-orchestration, knowledge, self-evolving skills, recovery, and observability.',

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
    ['link', { rel: 'icon', type: 'image/x-icon', href: '/favicon.ico' }],
    ['link', { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/favicon-32x32.png' }],
    ['link', { rel: 'icon', type: 'image/png', sizes: '16x16', href: '/favicon-16x16.png' }],
    ['link', { rel: 'apple-touch-icon', sizes: '180x180', href: '/apple-touch-icon.png' }],
    ['meta', { name: 'theme-color', content: '#c8b560' }],
    ['meta', { name: 'author', content: 'y-agent Team' }],
    [
      'meta',
      {
        name: 'keywords',
        content:
          'y-agent, agent harness, Rust, LLM, plan mode, goal, self-orchestration, multi-agent, DAG workflow, MCP, knowledge base, self-evolving skills, observability',
      },
    ],
    ['meta', { name: 'robots', content: 'index, follow' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'y-agent' }],
    [
      'meta',
      {
        property: 'og:title',
        content: 'y-agent -- Rust Agent Harness',
      },
    ],
    [
      'meta',
      {
        property: 'og:description',
        content:
          'A model-agnostic Agent Harness with planning, self-orchestration, knowledge, self-evolving skills, recovery, and observability.',
      },
    ],
    ['meta', { property: 'og:url', content: SITE_URL }],
    ['meta', { property: 'og:locale', content: 'en' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    [
      'meta',
      {
        name: 'twitter:title',
        content: 'y-agent -- Rust Agent Harness',
      },
    ],
    [
      'meta',
      {
        name: 'twitter:description',
        content:
          'Goal-directed planning, self-orchestration, knowledge, self-evolving skills, recovery, and observability.',
      },
    ],
  ],

  locales: {
    root: {
      label: 'English',
      lang: 'en',
      title: 'y-agent',
      description:
        'A Rust-first, model-agnostic Agent Harness.',
      themeConfig: {
        nav: [
          { text: 'Home', link: '/' },
          { text: 'Download', link: '/download' },
          { text: 'Docs', link: '/guide/getting-started' },
          { text: 'Development', link: '/development/' },
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
          '/development/': [
            {
              text: 'Development',
              items: [
                { text: 'Overview', link: '/development/' },
                {
                  text: 'Architecture',
                  link: '/development/architecture',
                },
                {
                  text: 'Observability',
                  link: '/development/observability',
                },
                {
                  text: 'Contributing',
                  link: '/development/contributing',
                },
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
    logo: '/logo-nav.png',
    socialLinks: [
      { icon: 'github', link: 'https://github.com/gorgiaxx/y-agent' },
    ],
    search: {
      provider: 'local',
    },
    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright 2026 y-agent Team',
    },
  },
}));
