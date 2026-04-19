import { describe, expect, it } from 'vitest';

import {
  applyGeneratedImageDelta,
  extractGeneratedImages,
  upsertGeneratedImage,
} from '../lib/generatedImages';

describe('generated image utilities', () => {
  it('appends streaming image deltas to the matching image slot', () => {
    const first = applyGeneratedImageDelta([], {
      index: 0,
      mime_type: 'image/png',
      partial_data: 'iVBOR',
    });
    const second = applyGeneratedImageDelta(first, {
      index: 0,
      mime_type: 'image/png',
      partial_data: 'w0KGgo=',
    });

    expect(second).toEqual([
      {
        index: 0,
        mime_type: 'image/png',
        data: 'iVBORw0KGgo=',
      },
    ]);
  });

  it('replaces complete image payloads and keeps images sorted by index', () => {
    const images = upsertGeneratedImage(
      [
        { index: 2, mime_type: 'image/webp', data: 'ccc' },
        { index: 0, mime_type: 'image/png', data: 'aaa' },
      ],
      { index: 1, mime_type: 'image/jpeg', data: 'bbb' },
    );

    expect(images.map((image) => image.index)).toEqual([0, 1, 2]);
    expect(images[1].data).toBe('bbb');
  });

  it('extracts persisted generated images from message metadata', () => {
    const images = extractGeneratedImages({
      generated_images: [
        {
          index: 1,
          mime_type: 'image/jpeg',
          data: 'bbb',
        },
        {
          index: 0,
          mime_type: 'image/png',
          data: 'aaa',
        },
      ],
    });

    expect(images).toEqual([
      { index: 0, mime_type: 'image/png', data: 'aaa' },
      { index: 1, mime_type: 'image/jpeg', data: 'bbb' },
    ]);
  });
});
