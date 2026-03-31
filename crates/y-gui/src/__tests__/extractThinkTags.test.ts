// ---------------------------------------------------------------------------
// Unit tests for extractThinkTags and escapeThinkTags
// ---------------------------------------------------------------------------

import { describe, it, expect } from 'vitest';
import { extractThinkTags, escapeThinkTags } from '../components/chat-panel/chat-box/messageUtils';

// ---------------------------------------------------------------------------
// extractThinkTags
// ---------------------------------------------------------------------------

describe('extractThinkTags', () => {
  it('returns null for content without think tags', () => {
    const result = extractThinkTags('Hello world');
    expect(result.thinkContent).toBeNull();
    expect(result.strippedContent).toBe('Hello world');
    expect(result.isThinkingIncomplete).toBe(false);
  });

  it('extracts a complete think block at the start', () => {
    const result = extractThinkTags('<think>reasoning here</think>Conclusion text');
    expect(result.thinkContent).toBe('reasoning here');
    expect(result.strippedContent).toBe('Conclusion text');
    expect(result.isThinkingIncomplete).toBe(false);
  });

  it('handles leading whitespace before <think>', () => {
    const result = extractThinkTags('  \n<think>reasoning here</think>Conclusion');
    expect(result.thinkContent).toBe('reasoning here');
    expect(result.strippedContent).toBe('Conclusion');
    expect(result.isThinkingIncomplete).toBe(false);
  });

  it('does NOT extract <think> when preceded by non-whitespace', () => {
    const result = extractThinkTags('prefix<think>reasoning</think>end');
    expect(result.thinkContent).toBeNull();
    expect(result.strippedContent).toBe('prefix<think>reasoning</think>end');
  });

  it('treats unclosed <think> as streaming (incomplete)', () => {
    const result = extractThinkTags('<think>partial reasoning');
    expect(result.thinkContent).toBe('partial reasoning');
    expect(result.strippedContent).toBe('');
    expect(result.isThinkingIncomplete).toBe(true);
  });

  it('returns null for empty streaming think content', () => {
    const result = extractThinkTags('<think>');
    expect(result.thinkContent).toBeNull();
    expect(result.isThinkingIncomplete).toBe(true);
  });

  it('ignores very short think content (likely false positive)', () => {
    const result = extractThinkTags('<think>/</think>rest');
    expect(result.thinkContent).toBeNull();
    expect(result.strippedContent).toBe('<think>/</think>rest');
  });

  it('strips the think block and preserves trailing content', () => {
    const result = extractThinkTags(
      '<think>Let me analyze this problem step by step</think>\n\nHere is the answer.',
    );
    expect(result.thinkContent).toBe('Let me analyze this problem step by step');
    expect(result.strippedContent).toBe('Here is the answer.');
    expect(result.isThinkingIncomplete).toBe(false);
  });

  it('trims whitespace in extracted think content', () => {
    const result = extractThinkTags('<think>  padded content  </think>answer');
    expect(result.thinkContent).toBe('padded content');
  });
});

// ---------------------------------------------------------------------------
// escapeThinkTags
// ---------------------------------------------------------------------------

describe('escapeThinkTags', () => {
  it('escapes <think> and </think> for safe markdown rendering', () => {
    expect(escapeThinkTags('<think>hello</think>')).toBe('&lt;think&gt;hello&lt;/think&gt;');
  });

  it('returns unchanged text when no think tags are present', () => {
    expect(escapeThinkTags('normal text')).toBe('normal text');
  });

  it('escapes multiple occurrences', () => {
    const input = '<think>a</think> and <think>b</think>';
    const output = escapeThinkTags(input);
    expect(output).toContain('&lt;think&gt;');
    expect(output).not.toContain('<think>');
  });
});
