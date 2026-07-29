import { toc } from '@lobehub/icons/es/toc';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ProviderIconImg } from '../components/common/ProviderIconPicker';

describe('ProviderIconImg', () => {
  it.each(['GeminiCLI', 'Kiro', 'Venice'])(
    'renders the current %s provider icon',
    (iconId) => {
      const html = renderToStaticMarkup(<ProviderIconImg iconId={iconId} size={20} />);

      expect(html).toContain('<svg');
    },
  );

  it('resolves provider icon identifiers case-insensitively', () => {
    const html = renderToStaticMarkup(<ProviderIconImg iconId="deepseek" />);

    expect(html).toContain('<svg');
  });

  it('renders every icon offered by the provider icon catalog', () => {
    const missingIcons = toc
      .filter(({ id }) => !renderToStaticMarkup(<ProviderIconImg iconId={id} />).includes('<svg'))
      .map(({ id }) => id);

    expect(missingIcons).toEqual([]);
  });
});
