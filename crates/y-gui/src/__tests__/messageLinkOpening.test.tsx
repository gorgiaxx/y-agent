import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

import { makeMarkdownComponents } from '../components/chat-panel/chat-box/messageUtils';

const openUrl = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));

vi.mock('../lib/platform', () => ({
  platform: {
    openUrl,
  },
}));

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(),
  },
  platform: {
    openUrl,
  },
}));

type AnchorRenderer = (
  props: React.AnchorHTMLAttributes<HTMLAnchorElement>,
) => React.ReactElement;

describe('message markdown link opening', () => {
  it('opens absolute web links through the platform layer instead of WebView navigation', () => {
    const components = makeMarkdownComponents({});
    const Anchor = (components as Record<string, unknown>).a as AnchorRenderer | undefined;

    expect(Anchor).toBeTypeOf('function');

    const element = Anchor?.({
      href: 'https://example.com/docs',
      children: 'docs',
    }) as React.ReactElement<React.AnchorHTMLAttributes<HTMLAnchorElement>>;
    const event = {
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    } as unknown as React.MouseEvent<HTMLAnchorElement>;

    element.props.onClick?.(event);

    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(openUrl).toHaveBeenCalledWith('https://example.com/docs');
  });
});
