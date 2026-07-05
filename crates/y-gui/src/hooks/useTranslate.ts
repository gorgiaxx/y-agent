// ---------------------------------------------------------------------------
// useTranslate -- translate text via the backend translate_text command.
//
// Extracted from InputArea to remove a transport.invoke call from the
// presentational layer.
// ---------------------------------------------------------------------------

import { useState, useCallback } from 'react';
import { transport, logger } from '../lib';
export interface UseTranslateReturn {
  translating: boolean;
  translate: (text: string) => Promise<string | null>;
}

export function useTranslate(): UseTranslateReturn {
  const [translating, setTranslating] = useState(false);

  const translate = useCallback(async (text: string): Promise<string | null> => {
    const trimmed = text.trim();
    if (!trimmed) return null;
    setTranslating(true);
    try {
      const translated = await transport.invoke<string>('translate_text', { text: trimmed });
      return translated;
    } catch (e) {
      logger.error('[useTranslate] error:', e);
      return null;
    } finally {
      setTranslating(false);
    }
  }, []);

  return { translating, translate };
}
