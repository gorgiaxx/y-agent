import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';
import {
  TOOL_TAG_MAX_LEN,
  truncateForTag,
} from '../components/chat-panel/chat-box/toolCallUtils';

describe('truncateForTag', () => {
  it('leaves short strings unchanged', () => {
    expect(truncateForTag('ls -la')).toBe('ls -la');
  });

  it('trims surrounding whitespace', () => {
    expect(truncateForTag('  hello  ')).toBe('hello');
  });

  it('truncates over-long strings to the shared max length with an ellipsis', () => {
    const out = truncateForTag('a'.repeat(200));
    expect(out).toHaveLength(TOOL_TAG_MAX_LEN);
    expect(out.endsWith('…')).toBe(true);
    expect(out).toBe(`${'a'.repeat(TOOL_TAG_MAX_LEN - 1)}…`);
  });

  it('honors a custom max length', () => {
    expect(truncateForTag('abcdef', 4)).toBe('abc…');
  });
});

describe('tool-call tag/title truncation in rendering', () => {
  it('truncates the ShellExec command in both the collapsed label and the hover title', () => {
    const longCommand = `echo ${'x'.repeat(200)}`;
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 's1', name: 'ShellExec', arguments: JSON.stringify({ command: longCommand }) }}
        status="success"
        result={JSON.stringify({ stdout: 'ok', stderr: '' })}
      />,
    );
    const truncated = truncateForTag(longCommand);
    expect(html).toContain(truncated);
    expect(html).toContain(`title="${truncated}"`);
    expect(html).not.toContain(longCommand);
  });

  it('truncates the FileRead path in the hover title', () => {
    const longPath = `/Users/rin/Projects/${'deep/'.repeat(40)}file.rs`;
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'f1', name: 'FileRead', arguments: JSON.stringify({ path: longPath }) }}
        status="success"
        result={JSON.stringify({ content: 'fn main() {}' })}
      />,
    );
    expect(html).toContain(truncateForTag(longPath));
    expect(html).not.toContain(longPath);
  });

  it('truncates the Grep pattern in the collapsed label', () => {
    const longPattern = 'pattern_'.repeat(20);
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'g1', name: 'Grep', arguments: JSON.stringify({ pattern: longPattern }) }}
        status="success"
      />,
    );
    expect(html).toContain(truncateForTag(longPattern));
    expect(html).not.toContain(longPattern);
  });

  it('truncates the Glob pattern in the collapsed label', () => {
    const longPattern = `**/${'seg_'.repeat(40)}*.ts`;
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'gl1', name: 'Glob', arguments: JSON.stringify({ pattern: longPattern }) }}
        status="success"
      />,
    );
    expect(html).toContain(truncateForTag(longPattern));
    expect(html).not.toContain(longPattern);
  });

  it('truncates the URL in the hover title', () => {
    const longUrl = `https://example.com/${'segment/'.repeat(30)}`;
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'u1', name: 'Browser', arguments: JSON.stringify({ action: 'navigate', url: longUrl }) }}
        status="success"
        result={JSON.stringify({ url: longUrl, title: '' })}
      />,
    );
    expect(html).toContain(truncateForTag(longUrl));
    expect(html).not.toContain(longUrl);
  });

  it('truncates the ToolSearch value in the collapsed label', () => {
    const longValue = 'query_term_'.repeat(15);
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'ts1', name: 'ToolSearch', arguments: JSON.stringify({ query: longValue }) }}
        status="success"
      />,
    );
    expect(html).toContain(truncateForTag(longValue));
    expect(html).not.toContain(longValue);
  });

  it('truncates the KnowledgeSearch query in the collapsed label', () => {
    const longQuery = 'knowledge_'.repeat(15);
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{ id: 'ks1', name: 'KnowledgeSearch', arguments: JSON.stringify({ query: longQuery }) }}
        status="success"
      />,
    );
    expect(html).toContain(truncateForTag(longQuery));
    expect(html).not.toContain(longQuery);
  });
});
