/// <reference types="node" />

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

describe('generated image gallery layout', () => {
  it('keeps generated thumbnails grouped from left to right', () => {
    const css = readFileSync(
      new URL('../components/chat-panel/chat-box/AssistantBubble.css', import.meta.url),
      'utf8',
    );
    const galleryRule = css.match(/\.generated-image-gallery\s*\{(?<body>[^}]*)\}/s)
      ?.groups?.body;

    expect(galleryRule).toContain(
      'grid-template-columns: repeat(auto-fill, var(--generated-image-thumb-size));',
    );
    expect(galleryRule).toContain('justify-content: start;');
    expect(galleryRule).not.toMatch(/\b1fr\b/);
  });
});
