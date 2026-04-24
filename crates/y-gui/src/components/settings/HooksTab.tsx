// ---------------------------------------------------------------------------
// HooksTab -- Hook System (Middleware & Event Bus) configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { HooksFormData } from './settingsTypes';
import { jsonToHooks } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { HOOKS_SCHEMA } from '../../utils/settingsSchemas';
import { Input, SettingsGroup, SettingsItem } from '../ui';

interface HooksTabProps {
  loadSection: (section: string) => Promise<string>;
  hooksForm: HooksFormData;
  setHooksForm: React.Dispatch<React.SetStateAction<HooksFormData>>;
  setDirtyHooks: React.Dispatch<React.SetStateAction<boolean>>;
  setRawHooksToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function HooksTab({
  loadSection,
  hooksForm,
  setHooksForm,
  setDirtyHooks,
  setRawHooksToml,
}: HooksTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await transport.invoke<any>('config_get');
      const json = allConfig?.hooks ?? {};
      setHooksForm(jsonToHooks(json));
      try {
        const raw = await loadSection('hooks');
        setRawHooksToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawHooksToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setHooksForm, setRawHooksToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, hooksForm as unknown as Record<string, unknown>, HOOKS_SCHEMA));
    }
    setRawMode(next);
  }, [hooksForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title section-title--flush">
            <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawHooksToml(val);
            setDirtyHooks(true);
          }}
          placeholder="No hooks.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Hook System">
          <SettingsItem title="Middleware Timeout (ms)" description="Per-middleware timeout.">
            <Input
              numeric
              type="number"
              min={1000}
              step={1000}
              className="w-[100px]"
              value={hooksForm.middleware_timeout_ms}
              onChange={(e) => { setHooksForm({ ...hooksForm, middleware_timeout_ms: Number(e.target.value) || 30000 }); setDirtyHooks(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Event Channel Capacity" description="Channel capacity per subscriber.">
            <Input
              numeric
              type="number"
              min={64}
              step={256}
              className="w-[100px]"
              value={hooksForm.event_channel_capacity}
              onChange={(e) => { setHooksForm({ ...hooksForm, event_channel_capacity: Number(e.target.value) || 1024 }); setDirtyHooks(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Max Subscribers" description="Max event subscribers.">
            <Input
              numeric
              type="number"
              min={1}
              max={1024}
              className="w-[100px]"
              value={hooksForm.max_subscribers}
              onChange={(e) => { setHooksForm({ ...hooksForm, max_subscribers: Number(e.target.value) || 64 }); setDirtyHooks(true); }}
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
