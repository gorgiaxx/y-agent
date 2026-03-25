// ---------------------------------------------------------------------------
// Unit tests for processStreamContent and segmentActions
// ---------------------------------------------------------------------------

import { describe, it, expect } from 'vitest';
import { processStreamContent } from '../hooks/useStreamContent';
import { segmentActions } from '../hooks/useActionSegment';

// ---------------------------------------------------------------------------
// processStreamContent
// ---------------------------------------------------------------------------

describe('processStreamContent', () => {
  it('returns null-like for plain text without tool_call tags', () => {
    // processStreamContent is only called when content contains tool_call tags
    // so we test with content that has those tags
    const result = processStreamContent('Hello world');
    expect(result.segments).toHaveLength(1);
    expect(result.segments[0].type).toBe('text');
    expect(result.toolCalls).toHaveLength(0);
    expect(result.hasPendingToolCall).toBe(false);
  });

  it('extracts a single complete tool call (XML-nested format)', () => {
    const content = 'Before text<tool_call>\n<name>read_file</name>\n<arguments>{"path": "/foo"}</arguments>\n</tool_call>After text';
    const result = processStreamContent(content);

    expect(result.toolCalls).toHaveLength(1);
    expect(result.toolCalls[0].name).toBe('read_file');
    expect(result.toolCalls[0].arguments).toContain('/foo');
    expect(result.segments.length).toBeGreaterThanOrEqual(2);
  });

  it('extracts tool call name from function attribute format (Llama/Qwen)', () => {
    const content = '<tool_call>\n<function=write_file>\n<parameter=path>/bar</parameter>\n</function>\n</tool_call>';
    const result = processStreamContent(content);
    expect(result.toolCalls).toHaveLength(1);
    expect(result.toolCalls[0].name).toBe('write_file');
  });

  it('detects pending (unclosed) tool call during streaming', () => {
    const content = 'prefix<tool_call>\n<name>search</name>\n<arguments>{"q": "test"';
    const result = processStreamContent(content);
    expect(result.hasPendingToolCall).toBe(true);
  });

  it('strips tool_result blocks from display content', () => {
    const content = 'Hello<tool_result>some result data</tool_result>World';
    const result = processStreamContent(content);
    expect(result.displayText).not.toContain('some result data');
    expect(result.displayText).not.toContain('tool_result');
  });

  it('handles multiple tool calls in sequence', () => {
    const content =
      'text1<tool_call>\n<name>tool_a</name>\n<arguments>{}</arguments>\n</tool_call>' +
      'text2<tool_call>\n<name>tool_b</name>\n<arguments>{}</arguments>\n</tool_call>text3';
    const result = processStreamContent(content);
    expect(result.toolCalls).toHaveLength(2);
    expect(result.toolCalls[0].name).toBe('tool_a');
    expect(result.toolCalls[1].name).toBe('tool_b');
  });
});

// ---------------------------------------------------------------------------
// segmentActions
// ---------------------------------------------------------------------------

describe('segmentActions', () => {
  it('returns empty result for segments with no tool calls', () => {
    const segments = [{ type: 'text' as const, text: 'Hello world' }];
    const result = segmentActions(segments);
    expect(result.actions).toHaveLength(0);
    expect(result.preamble).toHaveLength(0);
    expect(result.conclusion).toBeNull();
    expect(result.toolCallCount).toBe(0);
  });

  it('returns empty result for empty segments', () => {
    const result = segmentActions([]);
    expect(result.toolCallCount).toBe(0);
  });

  it('splits preamble, actions, and conclusion correctly', () => {
    const segments = [
      { type: 'text' as const, text: 'preamble text' },
      { type: 'tool_call' as const, toolCall: { name: 'read', arguments: '{}', startIndex: 0 } },
      { type: 'text' as const, text: 'interstitial' },
      { type: 'tool_call' as const, toolCall: { name: 'write', arguments: '{}', startIndex: 0 } },
      { type: 'text' as const, text: 'conclusion text' },
    ];
    const result = segmentActions(segments, false);
    expect(result.preamble).toHaveLength(1);
    expect(result.preamble[0].type).toBe('text');
    expect(result.actions).toHaveLength(3); // tool, text, tool
    expect(result.conclusion).not.toBeNull();
    expect(result.conclusion!.type).toBe('text');
    expect(result.toolCallCount).toBe(2);
  });

  it('does NOT create conclusion when hasPendingToolCall is true', () => {
    const segments = [
      { type: 'tool_call' as const, toolCall: { name: 'search', arguments: '{}', startIndex: 0 } },
      { type: 'text' as const, text: 'some trailing text' },
    ];
    const result = segmentActions(segments, true);
    expect(result.conclusion).toBeNull();
    expect(result.actions).toHaveLength(2); // tool + text (all as actions)
  });

  it('handles tool call as last segment (no conclusion)', () => {
    const segments = [
      { type: 'text' as const, text: 'preamble' },
      { type: 'tool_call' as const, toolCall: { name: 'run', arguments: '{}', startIndex: 0 } },
    ];
    const result = segmentActions(segments, false);
    expect(result.preamble).toHaveLength(1);
    expect(result.conclusion).toBeNull();
    expect(result.actions).toHaveLength(1);
  });
});
