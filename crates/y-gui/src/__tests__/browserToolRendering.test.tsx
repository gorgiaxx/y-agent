import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import {
  canonicalToolName,
  extractBrowserActionSummary,
  formatArguments,
  formatResultFormatted,
} from '../components/chat-panel/chat-box/toolCallUtils';
import { StaticBubble } from '../components/chat-panel/chat-box/StaticBubble';
import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';

describe('Browser tool rendering', () => {
  it('strips result-only fields from browser arguments display', () => {
    const formatted = formatArguments(
      'Browser',
      JSON.stringify({
        action: 'click',
        selector: '@e36',
        ok: true,
      }),
    );

    expect(formatted).toContain('"action": "click"');
    expect(formatted).toContain('"selector": "@e36"');
    expect(formatted).not.toContain('"ok": true');
  });

  it('formats repeated browser click results as a compact success state', () => {
    const formatted = formatResultFormatted(
      'Browser',
      JSON.stringify({
        action: 'click',
        ok: true,
        selector: '@e36',
      }),
      JSON.stringify({
        action: 'click',
        selector: '@e36',
      }),
    );

    expect(formatted).not.toBeNull();
    expect(formatted?.parts).toEqual([{ text: 'Success', isStderr: false }]);
  });

  it('extracts a compact action summary for browser click tool calls', () => {
    const summary = extractBrowserActionSummary(
      JSON.stringify({
        action: 'click',
        selector: '@e36',
      }),
    );

    expect(summary).toEqual({
      action: 'click',
      label: 'Click',
      detail: '@e36',
    });
  });

  it('normalizes lowercase browser tool names to the Browser renderer key', () => {
    expect(canonicalToolName('browser')).toBe('Browser');
  });

  it('renders browser click calls with the tag layout instead of the default tool card', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'browser-click-1',
          name: 'Browser',
          arguments: JSON.stringify({
            action: 'click',
            selector: '@e36',
          }),
        }}
        status="success"
        result={JSON.stringify({
          action: 'click',
          ok: true,
          selector: '@e36',
        })}
      />,
    );

    expect(html).toContain('tool-call-browser-tag');
    expect(html).toContain('Click');
    expect(html).toContain('@e36');
    expect(html).not.toContain('tool-call-card');
  });

  it('keeps browser navigate calls on the URL tag layout', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'browser-nav-1',
          name: 'Browser',
          arguments: JSON.stringify({
            action: 'navigate',
            url: 'https://tieba.baidu.com/p/8266195456',
          }),
        }}
        status="success"
        result={JSON.stringify({
          url: 'https://tieba.baidu.com/p/8266195456',
          title: '百度贴吧',
        })}
      />,
    );

    expect(html).toContain('tool-call-url-tag');
    expect(html).toContain('百度贴吧');
    expect(html).not.toContain('tool-call-browser-tag');
  });

  it('keeps browser failure rendering on the tag layout for lowercase streamed tool names', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'browser-error-1',
          name: 'browser',
          arguments: JSON.stringify({
            action: 'navigate',
            url: 'https://tieba.baidu.com/p/8266195456',
          }),
        }}
        status="error"
        result="browser navigation failed: net::ERR_ABORTED"
      />,
    );

    expect(html).toContain('tool-call-url-tag');
    expect(html).toContain('tieba.baidu.com');
    expect(html).not.toContain('tool-call-card');
  });

  it('keeps malformed failed browser calls on the browser tag layout', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'browser-error-2',
          name: 'Browser',
          arguments: '',
        }}
        status="error"
        result="browser click failed: target element is detached"
      />,
    );

    expect(html).toContain('tool-call-browser-tag');
    expect(html).toContain('Browser');
    expect(html).not.toContain('tool-call-card');
  });

  it('matches lowercase streamed browser calls with Browser error results', () => {
    const html = renderToStaticMarkup(
      <StaticBubble
        message={{
          id: 'message-1',
          role: 'assistant',
          content: '<tool_call><name>browser</name><arguments>{"action":"navigate","url":"https://tieba.baidu.com/p/8266195456"}</arguments></tool_call>',
          timestamp: '2026-04-13T00:00:00Z',
          tool_calls: [],
          metadata: {
            tool_results: [
              {
                name: 'Browser',
                arguments: '{"action":"navigate","url":"https://tieba.baidu.com/p/8266195456"}',
                success: false,
                duration_ms: 42,
                result_preview: 'browser navigation failed: net::ERR_ABORTED',
              },
            ],
          },
        }}
      />,
    );

    expect(html).toContain('tool-call-url-tag');
    expect(html).toContain('tool-status-error');
    expect(html).toContain('tieba.baidu.com');
  });
});
