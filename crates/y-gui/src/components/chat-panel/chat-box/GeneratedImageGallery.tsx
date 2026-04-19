import { useCallback, useState } from 'react';
import {
  Copy,
  Download,
  Expand,
  X,
} from 'lucide-react';

import type { GeneratedImage } from '../../../types';
import {
  generatedImageDataUrl,
  generatedImageFilename,
} from '../../../lib/generatedImages';

export interface GeneratedImageGalleryProps {
  images: GeneratedImage[];
}

async function copyGeneratedImage(image: GeneratedImage) {
  const dataUrl = generatedImageDataUrl(image);
  try {
    if (navigator.clipboard?.write && typeof ClipboardItem !== 'undefined') {
      const blob = await fetch(dataUrl).then((response) => response.blob());
      await navigator.clipboard.write([
        new ClipboardItem({
          [blob.type]: blob,
        }),
      ]);
      return;
    }
  } catch (error) {
    console.warn('[generated-image] binary clipboard copy failed:', error);
  }

  await navigator.clipboard?.writeText?.(dataUrl);
}

function downloadGeneratedImage(image: GeneratedImage) {
  const link = document.createElement('a');
  link.href = generatedImageDataUrl(image);
  link.download = generatedImageFilename(image);
  document.body.appendChild(link);
  link.click();
  link.remove();
}

export function GeneratedImageGallery({ images }: GeneratedImageGalleryProps) {
  const [previewImage, setPreviewImage] = useState<GeneratedImage | null>(null);

  const handlePreview = useCallback((image: GeneratedImage) => {
    setPreviewImage(image);
  }, []);

  const handleClosePreview = useCallback(() => {
    setPreviewImage(null);
  }, []);

  if (images.length === 0) {
    return null;
  }

  return (
    <>
      <div className="generated-image-gallery">
        {images.map((image) => {
          const src = generatedImageDataUrl(image);
          const label = `Generated image ${image.index + 1}`;
          return (
            <figure className="generated-image-card" key={`generated-image-${image.index}`}>
              <button
                className="generated-image-preview"
                onClick={() => handlePreview(image)}
                title={`Preview ${label.toLowerCase()}`}
                type="button"
              >
                <img
                  src={src}
                  alt={label}
                  className="generated-image-thumb"
                />
              </button>
              <figcaption className="generated-image-actions">
                <button
                  className="generated-image-action"
                  onClick={() => void copyGeneratedImage(image)}
                  title="Copy image"
                  type="button"
                >
                  <Copy size={14} />
                  <span>Copy</span>
                </button>
                <button
                  className="generated-image-action"
                  onClick={() => downloadGeneratedImage(image)}
                  title="Download image"
                  type="button"
                >
                  <Download size={14} />
                  <span>Download</span>
                </button>
                <button
                  className="generated-image-action"
                  onClick={() => handlePreview(image)}
                  title="Preview image"
                  type="button"
                >
                  <Expand size={14} />
                  <span>Preview</span>
                </button>
              </figcaption>
            </figure>
          );
        })}
      </div>

      {previewImage && (
        <div
          className="generated-image-lightbox"
          onClick={handleClosePreview}
          role="presentation"
        >
          <div
            className="generated-image-lightbox-dialog"
            onClick={(event) => event.stopPropagation()}
            role="dialog"
            aria-modal="true"
            aria-label={`Preview generated image ${previewImage.index + 1}`}
          >
            <button
              className="generated-image-lightbox-close"
              onClick={handleClosePreview}
              title="Close preview"
              type="button"
            >
              <X size={16} />
            </button>
            <img
              src={generatedImageDataUrl(previewImage)}
              alt={`Generated image ${previewImage.index + 1}`}
              className="generated-image-lightbox-image"
            />
          </div>
        </div>
      )}
    </>
  );
}
