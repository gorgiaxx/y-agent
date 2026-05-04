import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

function readSource(path: string): string {
  return readFileSync(new URL(path, import.meta.url), 'utf8');
}

describe('bundle boundaries', () => {
  it('keeps Monaco implementation imports behind the lazy editor boundary', () => {
    const wrapper = readSource('../components/ui/MonacoEditor.tsx');

    expect(wrapper).not.toContain('react-monaco-editor');
    expect(wrapper).not.toContain('monaco-editor/esm');
    expect(wrapper).toContain('lazy(');
  });

  it('loads Mermaid only when a diagram block is rendered', () => {
    const source = readSource('../components/chat-panel/chat-box/MermaidBlock.tsx');

    expect(source).not.toContain("import mermaid from 'mermaid'");
    expect(source).toContain("import('mermaid')");
  });

  it('loads syntax highlighting only when a code block is rendered', () => {
    const source = readSource('../components/chat-panel/chat-box/MessageShared.tsx');

    expect(source).not.toContain("from 'react-syntax-highlighter'");
    expect(source).toContain("import('react-syntax-highlighter')");
  });
});
