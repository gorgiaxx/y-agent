// ---------------------------------------------------------------------------
// useInputAttachments -- manage image attachments via paste and file picker.
//
// Extracted from InputArea to remove transport.invoke / platform calls from
// the presentational layer. Handles clipboard paste and the native file-open
// dialog (including browser fallback via platform.openImageAttachments).
// ---------------------------------------------------------------------------

import { useState, useCallback } from 'react';
import { transport, platform, logger } from '../lib';
import type { Attachment } from '../types';

const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp'];
const IMAGE_FILTERS = [{ name: 'Images', extensions: IMAGE_EXTENSIONS }];

export interface UseInputAttachmentsReturn {
  attachments: Attachment[];
  addAttachment: (att: Attachment) => void;
  removeAttachment: (id: string) => void;
  clearAttachments: () => void;
  /** Handle a paste event; extracts image files and returns true if handled. */
  handlePaste: (e: React.ClipboardEvent) => Promise<boolean>;
  /** Open the native file picker and add selected images. */
  pickFiles: () => Promise<void>;
}

export function useInputAttachments(): UseInputAttachmentsReturn {
  const [attachments, setAttachments] = useState<Attachment[]>([]);

  const addAttachment = useCallback((att: Attachment) => {
    setAttachments((prev) => [...prev, att]);
  }, []);

  const removeAttachment = useCallback((id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  }, []);

  const clearAttachments = useCallback(() => {
    setAttachments([]);
  }, []);

  const handlePaste = useCallback(async (e: React.ClipboardEvent): Promise<boolean> => {
    const items = e.clipboardData.items;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) return false;
        const buffer = await file.arrayBuffer();
        const base64 = btoa(
          new Uint8Array(buffer).reduce((data, byte) => data + String.fromCharCode(byte), ''),
        );
        const ext = file.type.split('/')[1] || 'png';
        const att: Attachment = {
          id: `paste-${Date.now()}`,
          filename: `pasted-image.${ext}`,
          mime_type: file.type,
          base64_data: base64,
          size: file.size,
        };
        setAttachments((prev) => [...prev, att]);
        return true;
      }
    }
    return false;
  }, []);

  const pickFiles = useCallback(async () => {
    try {
      if (!platform.capabilities.nativeFilePaths) {
        const atts = await platform.openImageAttachments({
          multiple: true,
          filters: IMAGE_FILTERS,
        });
        if (atts) {
          setAttachments((prev) => [...prev, ...atts]);
        }
        return;
      }
      const result = await platform.openFileDialog({
        multiple: true,
        filters: IMAGE_FILTERS,
      });
      if (result) {
        const atts = await transport.invoke<Attachment[]>('attachment_read_files', { paths: result });
        setAttachments((prev) => [...prev, ...atts]);
      }
    } catch (e) {
      logger.error('[useInputAttachments] pick error:', e);
    }
  }, []);

  return { attachments, addAttachment, removeAttachment, clearAttachments, handlePaste, pickFiles };
}
