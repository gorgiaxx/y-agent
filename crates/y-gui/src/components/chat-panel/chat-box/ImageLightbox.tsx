import { useCallback, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';

export interface ImageLightboxProps {
  src: string;
  alt: string;
  onClose: () => void;
}

/**
 * Shared full-screen image lightbox rendered via portal to document.body.
 *
 * Reused by generated-image previews and user-attachment previews so the
 * overlay is never clipped by `contain`/`overflow` on ancestor bubbles.
 */
export function ImageLightbox({ src, alt, onClose }: ImageLightboxProps) {
  const handleClose = useCallback(() => onClose(), [onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [handleClose]);

  return createPortal(
    <div
      className="generated-image-lightbox"
      onClick={handleClose}
      role="presentation"
    >
      <img src={src} alt={alt} className="generated-image-lightbox-image" />
      <button
        type="button"
        className="generated-image-lightbox-close"
        onClick={(e) => {
          e.stopPropagation();
          handleClose();
        }}
        aria-label="Close preview"
      >
        <X size={16} />
      </button>
    </div>,
    document.body,
  );
}
