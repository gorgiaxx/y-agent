import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

import { StatusBar } from '../components/chat-panel/StatusBar';

vi.mock('../components/common/ProviderIconPicker', () => ({
  ProviderIconImg: () => null,
}));

describe('StatusBar token ratio display', () => {
  it('renders both numerator and denominator with units when numerator is small', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        contextWindow={1_000_000}
        contextTokensUsed={300}
      />,
    );

    expect(html).not.toMatch(/>\s*300\/1\.0M\s*</);
    expect(html).toContain('0.30k/1.0M');
  });

  it('renders the numerator with a unit even when below 1k', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        contextWindow={200_000}
        contextTokensUsed={42}
      />,
    );

    expect(html).toMatch(/0\.0\dk\/200\.0k/);
  });

  it('keeps the k/M scaling for larger numerators', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        contextWindow={1_000_000}
        contextTokensUsed={150_000}
      />,
    );

    expect(html).toContain('150.0k/1.0M');
  });

  it('exposes the raw token counts via tooltip to remove ambiguity', () => {
    const html = renderToStaticMarkup(
      <StatusBar
        version="debug"
        contextWindow={1_000_000}
        contextTokensUsed={300}
      />,
    );

    expect(html).toContain('title="300 / 1,000,000 tokens');
  });
});
