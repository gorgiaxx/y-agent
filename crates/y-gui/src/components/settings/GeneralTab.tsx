// ---------------------------------------------------------------------------
// GeneralTab -- Paths, Appearance, Behavior settings
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Copy } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import type { GuiConfig } from '../../types';

interface GeneralTabProps {
  localConfig: GuiConfig;
  setLocalConfig: (config: GuiConfig) => void;
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
}

export function GeneralTab({ localConfig, setLocalConfig, setToast }: GeneralTabProps) {
  const [configPath, setConfigPath] = useState('');
  const [dataPath, setDataPath] = useState('');
  const [copiedField, setCopiedField] = useState<string | null>(null);

  useEffect(() => {
    invoke<{ config_dir: string; data_dir: string }>('app_paths')
      .then((paths) => {
        setConfigPath(paths.config_dir);
        setDataPath(paths.data_dir);
      })
      .catch(() => { /* ignore */ });
  }, []);

  const handleCopyPath = useCallback(async (value: string, field: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 1500);
      setToast({ message: 'Copied to clipboard', type: 'success' });
    } catch { /* ignore */ }
  }, [setToast]);

  return (
    <div className="settings-section">
      <h3 className="section-title">Paths</h3>
      <div className="form-group">
        <label className="form-label">Config Directory</label>
        <div className="path-field-wrap">
          <input
            className="pf-input path-field-input"
            value={configPath}
            readOnly
            title={configPath}
          />
          <button
            className="path-field-copy"
            onClick={() => handleCopyPath(configPath, 'config')}
            title={copiedField === 'config' ? 'Copied!' : 'Copy path'}
            type="button"
          >
            <Copy size={13} />
          </button>
        </div>
      </div>
      <div className="form-group">
        <label className="form-label">Data Directory</label>
        <div className="path-field-wrap">
          <input
            className="pf-input path-field-input"
            value={dataPath}
            readOnly
            title={dataPath}
          />
          <button
            className="path-field-copy"
            onClick={() => handleCopyPath(dataPath, 'data')}
            title={copiedField === 'data' ? 'Copied!' : 'Copy path'}
            type="button"
          >
            <Copy size={13} />
          </button>
        </div>
      </div>

      <h3 className="section-title">Appearance</h3>

      <div className="form-group">
        <label className="form-label">Theme</label>
        <select
          className="form-select"
          value={localConfig.theme}
          onChange={(e) =>
            setLocalConfig({ ...localConfig, theme: e.target.value as GuiConfig['theme'] })
          }
        >
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="system">System</option>
        </select>
      </div>

      <div className="form-group">
        <label className="form-label">Font Size</label>
        <div className="form-range-group">
          <input
            type="range"
            className="form-range"
            min="12"
            max="20"
            value={localConfig.font_size}
            onChange={(e) =>
              setLocalConfig({ ...localConfig, font_size: Number(e.target.value) })
            }
          />
          <span className="range-value">{localConfig.font_size}px</span>
        </div>
      </div>

      <h3 className="section-title">Behavior</h3>

      <div className="form-group">
        <label className="form-label">
          <input
            type="checkbox"
            className="form-checkbox"
            checked={localConfig.send_on_enter}
            onChange={(e) =>
              setLocalConfig({ ...localConfig, send_on_enter: e.target.checked })
            }
          />
          Send message on Enter
        </label>
        <p className="form-hint">
          When enabled, press Enter to send and Shift+Enter for newline.
        </p>
      </div>
    </div>
  );
}
