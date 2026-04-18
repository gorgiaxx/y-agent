// Custom hook for GUI configuration.

import { useState, useCallback, useEffect } from 'react';
import { transport } from '../lib';
import type { GuiConfig } from '../types';

const defaultConfig: GuiConfig = {
  theme: 'dark',
  font_size: 14,
  send_on_enter: true,
  window_width: 1200,
  window_height: 800,
  setup_completed: false,
  translate_target_language: '',
  use_custom_decorations: false,
};

interface UseConfigReturn {
  config: GuiConfig;
  updateConfig: (updates: Partial<GuiConfig>) => Promise<void>;
  loading: boolean;
  loadSection: (section: string) => Promise<string>;
  saveSection: (section: string, content: string) => Promise<void>;
  reloadConfig: () => Promise<string>;
}

export function useConfig(): UseConfigReturn {
  const [config, setConfig] = useState<GuiConfig>(defaultConfig);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const load = async () => {
      try {
        const cfg = await transport.invoke<GuiConfig>('config_get_gui');
        setConfig(cfg);
      } catch (e) {
        console.error('Failed to load GUI config:', e);
      } finally {
        setLoading(false);
      }
    };
    load();
  }, []);

  // Apply font size whenever config changes.
  // (Theme application is handled by useTheme hook.)
  useEffect(() => {
    document.documentElement.style.fontSize = `${config.font_size}px`;
  }, [config.font_size]);

  const updateConfig = useCallback(
    async (updates: Partial<GuiConfig>) => {
      const newConfig = { ...config, ...updates };
      try {
        await transport.invoke('config_set_gui', { config: newConfig });
        setConfig(newConfig);
      } catch (e) {
        console.error('Failed to save GUI config:', e);
      }
    },
    [config],
  );

  const loadSection = useCallback(async (section: string): Promise<string> => {
    return await transport.invoke<string>('config_get_section', { section });
  }, []);

  const saveSection = useCallback(async (section: string, content: string): Promise<void> => {
    await transport.invoke('config_save_section', { section, content });
  }, []);

  const reloadConfig = useCallback(async (): Promise<string> => {
    return await transport.invoke<string>('config_reload');
  }, []);

  return { config, updateConfig, loading, loadSection, saveSection, reloadConfig };
}
