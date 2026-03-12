// Custom hook for GUI configuration.

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { GuiConfig } from '../types';

const defaultConfig: GuiConfig = {
  theme: 'dark',
  font_size: 14,
  send_on_enter: true,
  window_width: 1200,
  window_height: 800,
};

interface UseConfigReturn {
  config: GuiConfig;
  updateConfig: (updates: Partial<GuiConfig>) => Promise<void>;
  loading: boolean;
}

export function useConfig(): UseConfigReturn {
  const [config, setConfig] = useState<GuiConfig>(defaultConfig);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const load = async () => {
      try {
        const cfg = await invoke<GuiConfig>('config_get_gui');
        setConfig(cfg);
      } catch (e) {
        console.error('Failed to load GUI config:', e);
      } finally {
        setLoading(false);
      }
    };
    load();
  }, []);

  // Apply theme whenever config changes.
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', config.theme);
    document.documentElement.style.fontSize = `${config.font_size}px`;
  }, [config.theme, config.font_size]);

  const updateConfig = useCallback(
    async (updates: Partial<GuiConfig>) => {
      const newConfig = { ...config, ...updates };
      try {
        await invoke('config_set_gui', { config: newConfig });
        setConfig(newConfig);
      } catch (e) {
        console.error('Failed to save GUI config:', e);
      }
    },
    [config],
  );

  return { config, updateConfig, loading };
}
