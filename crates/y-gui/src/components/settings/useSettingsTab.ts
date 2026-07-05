// ---------------------------------------------------------------------------
// useSettingsTab -- shared hook for TOML-backed settings form tabs.
//
// Eliminates the boilerplate duplicated across the 9 canonical settings tabs
// (Session, Runtime, Browser, Storage, Hooks, Tools, Guardrails, Knowledge,
// Langfuse): loading state, raw-mode toggle, raw-content cache, the load-on-mount
// effect, and the per-field dirty helper.
//
// Each tab supplies:
//   - section:     the TOML section name ('session', 'runtime', ...)
//   - schema:      the FieldDef[] describing how form fields map to TOML keys
//   - configKey:   the key to pluck from the AppConfigResponse JSON
//   - form / setForm / setDirty / setRawToml: lifted state from SettingsPanel
//   - jsonToForm:  deserializes the JSON slice into the form type
//   - loadSection: raw TOML reader (from useConfig)
//   - onLoaded?:   optional callback for tabs that do extra work after load
//                  (e.g. SessionTab also loads the retry form)
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { AppConfigResponse } from '../../types';
import { mergeIntoRawToml, type FieldDef } from '../../utils/tomlUtils';

export interface UseSettingsTabOptions<TForm> {
  section: string;
  schema: FieldDef[];
  configKey: keyof AppConfigResponse;
  form: TForm;
  setForm: React.Dispatch<React.SetStateAction<TForm>>;
  setDirty: React.Dispatch<React.SetStateAction<boolean>>;
  setRawToml: React.Dispatch<React.SetStateAction<string | undefined>>;
  jsonToForm: (json: unknown) => TForm;
  loadSection: (section: string) => Promise<string>;
  /** Invoked after a successful load with the full AppConfigResponse, for
   *  tabs that hydrate additional state from the same fetch (e.g. retry form). */
  onLoaded?: (allConfig: AppConfigResponse) => void;
}

export interface UseSettingsTabResult<TForm> {
  loading: boolean;
  rawMode: boolean;
  rawContent: string;
  handleToggleRaw: (next: boolean) => void;
  handleRawChange: (val: string) => void;
  /** Update one or more form fields and mark the section dirty. */
  update: (patch: Partial<TForm>) => void;
}

export function useSettingsTab<TForm>(
  opts: UseSettingsTabOptions<TForm>,
): UseSettingsTabResult<TForm> {
  const {
    section,
    schema,
    configKey,
    form,
    setForm,
    setDirty,
    setRawToml,
    jsonToForm,
    loadSection,
    onLoaded,
  } = opts;

  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);

  // Keep onLoaded in a ref so the load effect doesn't re-run when the caller's
  // callback identity changes (it captures lifted setters which are stable).
  const onLoadedRef = useRef(onLoaded);
  onLoadedRef.current = onLoaded;

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      const allConfig = await transport.invoke<AppConfigResponse>('config_get');
      const json = allConfig?.[configKey] ?? {};
      setForm(jsonToForm(json));
      onLoadedRef.current?.(allConfig);
      // Cache raw TOML for comment preservation and seed the raw editor so
      // switching to raw mode shows on-disk content before any form merge.
      try {
        const raw = await loadSection(section);
        setRawToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if section not found.
    } finally {
      setLoading(false);
    }
  }, [configKey, jsonToForm, loadSection, section, setForm, setRawToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      // Form -> Raw: merge form data into cached raw TOML to preserve comments.
      setRawContent(
        mergeIntoRawToml(cachedRawToml.current, form as unknown as Record<string, unknown>, schema),
      );
    }
    setRawMode(next);
  }, [form, schema]);

  const handleRawChange = useCallback((val: string) => {
    setRawContent(val);
    setRawToml(val);
    setDirty(true);
  }, [setRawToml, setDirty]);

  const update = useCallback((patch: Partial<TForm>) => {
    setForm((prev) => ({ ...prev, ...patch }));
    setDirty(true);
  }, [setForm, setDirty]);

  return { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update };
}
