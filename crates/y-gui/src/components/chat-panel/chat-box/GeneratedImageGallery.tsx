import { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import type { GeneratedImage } from '../../../types';
import { generatedImageDataUrl } from '../../../lib/generatedImages';

export interface GeneratedImageGalleryProps {
  images: GeneratedImage[];
}

export function GeneratedImageGallery({ images }: GeneratedImageGalleryProps) {
  const [previewIdx, setPreviewIdx] = useState<number | null>(null);

  const previewImage = previewIdx !== null ? images[previewIdx] ?? null : null;

  const handlePreview = useCallback((idx: number) => {
    setPreviewIdx(idx);
  }, []);

  const handleClosePreview = useCallback(() => {
    setPreviewIdx(null);
  }, []);

  const handlePrev = useCallback(() => {
    setPreviewIdx((prev) => (prev !== null && prev > 0 ? prev - 1 : prev));
  }, []);

  const handleNext = useCallback(() => {
    setPreviewIdx((prev) =>
      prev !== null && prev < images.length - 1 ? prev + 1 : prev,
    );
  }, [images.length]);

  useEffect(() => {
    if (previewIdx === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleClosePreview();
      else if (e.key === 'ArrowLeft') handlePrev();
      else if (e.key === 'ArrowRight') handleNext();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [previewIdx, handleClosePreview, handlePrev, handleNext]);

  if (images.length === 0) {
    return null;
  }

  return (
    <>
      <div className="generated-image-gallery">
        {images.map((image, idx) => {
          const src = generatedImageDataUrl(image);
          const label = `Generated image ${image.index + 1}`;
          return (
            <figure className="generated-image-card" key={`generated-image-${image.index}`}>
              <button
                className="generated-image-preview"
                onClick={() => handlePreview(idx)}
                title={`Preview ${label.toLowerCase()}`}
                type="button"
              >
                <img
                  src={src}
                  alt={label}
                  className="generated-image-thumb"
                />
              </button>
            </figure>
          );
        })}
      </div>

      {previewImage && previewIdx !== null && createPortal(
        <div
          className="generated-image-lightbox"
          onClick={handleClosePreview}
          role="presentation"
        >
          <img
            src={generatedImageDataUrl(previewImage)}
            alt={`Generated image ${previewImage.index + 1}`}
            className="generated-image-lightbox-image"
          />
        </div>,
        document.body,
      )}
    </>
  );
}
