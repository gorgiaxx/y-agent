import type { GeneratedImage } from '../types';

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null;
}

function normalizeGeneratedImages(images: GeneratedImage[]): GeneratedImage[] {
  return [...images].sort((a, b) => a.index - b.index);
}

export function upsertGeneratedImage(
  images: GeneratedImage[],
  next: GeneratedImage,
): GeneratedImage[] {
  const existingIndex = images.findIndex((image) => image.index === next.index);
  if (existingIndex >= 0) {
    const updated = [...images];
    updated[existingIndex] = next;
    return normalizeGeneratedImages(updated);
  }
  return normalizeGeneratedImages([...images, next]);
}

export function mergeGeneratedImages(
  base: GeneratedImage[],
  incoming: GeneratedImage[],
): GeneratedImage[] {
  let merged = [...base];
  for (const image of incoming) {
    merged = upsertGeneratedImage(merged, image);
  }
  return merged;
}

export function applyGeneratedImageDelta(
  images: GeneratedImage[],
  delta: {
    index: number;
    mime_type: string;
    partial_data: string;
  },
): GeneratedImage[] {
  const existing = images.find((image) => image.index === delta.index);
  const next: GeneratedImage = {
    index: delta.index,
    mime_type: delta.mime_type,
    data: `${existing?.data ?? ''}${delta.partial_data}`,
  };
  return upsertGeneratedImage(images, next);
}

export function extractGeneratedImages(metadata: unknown): GeneratedImage[] {
  if (!isRecord(metadata)) return [];
  const rawImages = metadata.generated_images;
  if (!Array.isArray(rawImages)) return [];

  const images = rawImages.flatMap((value): GeneratedImage[] => {
    if (!isRecord(value)) return [];
    if (
      typeof value.index !== 'number'
      || typeof value.mime_type !== 'string'
      || typeof value.data !== 'string'
    ) {
      return [];
    }
    return [{
      index: value.index,
      mime_type: value.mime_type,
      data: value.data,
    }];
  });

  return normalizeGeneratedImages(images);
}

export function generatedImageDataUrl(image: GeneratedImage): string {
  return `data:${image.mime_type};base64,${image.data}`;
}

export function generatedImageExtension(mimeType: string): string {
  switch (mimeType) {
    case 'image/jpeg':
      return 'jpg';
    case 'image/png':
      return 'png';
    case 'image/webp':
      return 'webp';
    case 'image/gif':
      return 'gif';
    default:
      return 'img';
  }
}

export function generatedImageFilename(image: GeneratedImage): string {
  return `generated-image-${image.index + 1}.${generatedImageExtension(image.mime_type)}`;
}
