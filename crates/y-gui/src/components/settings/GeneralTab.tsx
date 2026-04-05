// ---------------------------------------------------------------------------
// GeneralTab -- Paths, Appearance, Behavior settings
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Copy, Wand2 } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import type { GuiConfig } from '../../types';
import {
  Input,
  Button,
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
  SelectValue,
  Switch,
} from '../ui';

interface GeneralTabProps {
  localConfig: GuiConfig;
  setLocalConfig: (config: GuiConfig) => void;
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
  onRunWizard?: () => void;
}

export function GeneralTab({ localConfig, setLocalConfig, setToast, onRunWizard }: GeneralTabProps) {
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
        <div className="flex relative items-center">
          <Input
            variant="mono"
            value={configPath}
            readOnly
            title={configPath}
            className="pr-9 text-[var(--text-secondary)] cursor-default select-all"
          />
          <Button
            variant="icon"
            size="sm"
            className="absolute right-1"
            onClick={() => handleCopyPath(configPath, 'config')}
            title={copiedField === 'config' ? 'Copied!' : 'Copy path'}
          >
            <Copy size={13} />
          </Button>
        </div>
      </div>
      <div className="form-group">
        <label className="form-label">Data Directory</label>
        <div className="flex relative items-center">
          <Input
            variant="mono"
            value={dataPath}
            readOnly
            title={dataPath}
            className="pr-9 text-[var(--text-secondary)] cursor-default select-all"
          />
          <Button
            variant="icon"
            size="sm"
            className="absolute right-1"
            onClick={() => handleCopyPath(dataPath, 'data')}
            title={copiedField === 'data' ? 'Copied!' : 'Copy path'}
          >
            <Copy size={13} />
          </Button>
        </div>
      </div>

      <h3 className="section-title">Appearance</h3>

      <div className="form-group">
        <label className="form-label">Theme</label>
        <Select
          value={localConfig.theme}
          onValueChange={(v) =>
            setLocalConfig({ ...localConfig, theme: v as GuiConfig['theme'] })
          }
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="dark">Dark</SelectItem>
            <SelectItem value="light">Light</SelectItem>
            <SelectItem value="system">System</SelectItem>
          </SelectContent>
        </Select>
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
        <Switch
          label="Send message on Enter"
          checked={localConfig.send_on_enter}
          onCheckedChange={(checked) =>
            setLocalConfig({ ...localConfig, send_on_enter: checked })
          }
        />
        <p className="form-hint">
          When enabled, press Enter to send and Shift+Enter for newline.
        </p>
      </div>

      <h3 className="section-title">Setup</h3>

      <div className="form-group">
        <p className="form-hint" style={{ marginBottom: '8px' }}>
          Re-run the initial setup wizard to reconfigure providers, runtime, browser, guardrails, and knowledge base.
        </p>
        <Button
          variant="outline"
          onClick={onRunWizard}
        >
          <Wand2 size={14} />
          Run Setup Wizard
        </Button>
      </div>
    </div>
  );
}
