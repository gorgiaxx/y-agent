// ---------------------------------------------------------------------------
// GeneralTab -- Paths, Appearance, Behavior settings
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { Copy, Wand2 } from 'lucide-react';
import { transport } from '../../lib';
import type { GuiConfig } from '../../types';
import {
  Button,
  Input,
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
  SelectValue,
  Switch,
  SettingsGroup,
  SettingsItem,
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
  const copyTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    transport.invoke<{ config_dir: string; data_dir: string }>('app_paths')
      .then((paths) => {
        setConfigPath(paths.config_dir);
        setDataPath(paths.data_dir);
      })
      .catch(() => { /* ignore */ });
    return () => clearTimeout(copyTimerRef.current);
  }, []);

  const handleCopyPath = useCallback(async (value: string, field: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopiedField(null), 1500);
      setToast({ message: 'Copied to clipboard', type: 'success' });
    } catch { /* ignore */ }
  }, [setToast]);

  return (
    <div className="settings-section">
      <SettingsGroup title="Paths">
        <SettingsItem title="Config Directory" wide>
          <div className="flex relative items-center w-full">
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
        </SettingsItem>
        <SettingsItem title="Data Directory" wide>
          <div className="flex relative items-center w-full">
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
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Appearance">
        <SettingsItem title="Theme">
          <Select
            value={localConfig.theme}
            onValueChange={(v) =>
              setLocalConfig({ ...localConfig, theme: v as GuiConfig['theme'] })
            }
          >
            <SelectTrigger className="w-[140px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="dark">Dark</SelectItem>
              <SelectItem value="light">Light</SelectItem>
              <SelectItem value="system">System</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
        <SettingsItem title="Font Size">
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
        </SettingsItem>
        <SettingsItem
          title="Custom window decorations"
          description="Hide the native titlebar and render an Apple-style layered chrome. Recommended on macOS."
        >
          <Switch
            checked={localConfig.use_custom_decorations}
            onCheckedChange={(checked) =>
              setLocalConfig({ ...localConfig, use_custom_decorations: checked })
            }
          />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Behavior">
        <SettingsItem
          title="Send message on Enter"
          description="When enabled, press Enter to send and Shift+Enter for newline."
        >
          <Switch
            checked={localConfig.send_on_enter}
            onCheckedChange={(checked) =>
              setLocalConfig({ ...localConfig, send_on_enter: checked })
            }
          />
        </SettingsItem>
      </SettingsGroup>
      <SettingsGroup title="Translation">
        <SettingsItem
          title="Default Target Language"
          description="The target language used by the translate button in the chat input toolbar."
        >
          <Select
            value={localConfig.translate_target_language}
            onValueChange={(v) =>
              setLocalConfig({ ...localConfig, translate_target_language: v })
            }
          >
            <SelectTrigger className="w-[140px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="Chinese">Chinese</SelectItem>
              <SelectItem value="English">English</SelectItem>
              <SelectItem value="Japanese">Japanese</SelectItem>
              <SelectItem value="Korean">Korean</SelectItem>
              <SelectItem value="Spanish">Spanish</SelectItem>
              <SelectItem value="Russian">Russian</SelectItem>
              <SelectItem value="German">German</SelectItem>
              <SelectItem value="French">French</SelectItem>
              <SelectItem value="Italian">Italian</SelectItem>
            </SelectContent>
          </Select>
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup
        title="Setup"
        description="Re-run the initial setup wizard to reconfigure providers, runtime, browser, guardrails, and knowledge base."
      >
        <SettingsItem title="Setup Wizard">
          <Button variant="outline" onClick={onRunWizard}>
            <Wand2 size={14} />
            Run Setup Wizard
          </Button>
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}