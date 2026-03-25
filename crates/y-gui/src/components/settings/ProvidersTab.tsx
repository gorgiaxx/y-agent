// ---------------------------------------------------------------------------
// ProvidersTab -- Provider list sidebar + ProviderTabPanel detail form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { Eye, EyeOff, Plus, X, Bot, Copy, ChevronUp, ChevronDown, Search } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { ProviderIconPicker, ProviderIconImg } from '../common/ProviderIconPicker';
import { TagChipInput } from './TagChipInput';
import type { ProviderFormData } from './settingsTypes';
import { emptyProvider, jsonToProviders, providersToToml } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';

// ---------------------------------------------------------------------------
// Model item returned from /v1/models
// ---------------------------------------------------------------------------

interface ModelItem {
  id: string;
  display_name?: string;
}

// ---------------------------------------------------------------------------
// ModelPickerDropdown -- filterable model list overlay
// ---------------------------------------------------------------------------

function ModelPickerDropdown({
  models,
  loading,
  error,
  onSelect,
  onClose,
}: {
  models: ModelItem[];
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = useState('');
  const dropdownRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus the filter input when dropdown opens.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Close on click outside.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [onClose]);

  const filtered = models.filter((m) =>
    m.id.toLowerCase().includes(filter.toLowerCase()) ||
    (m.display_name ?? '').toLowerCase().includes(filter.toLowerCase()),
  );

  return (
    <div className="model-picker-dropdown" ref={dropdownRef}>
      <div className="model-picker-search">
        <Search size={12} className="model-picker-search-icon" />
        <input
          ref={inputRef}
          className="model-picker-filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter models..."
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
          }}
        />
      </div>
      {loading && (
        <div className="model-picker-status">
          <span className="pf-spinner" /> Fetching models...
        </div>
      )}
      {error && (
        <div className="model-picker-status model-picker-error">{error}</div>
      )}
      {!loading && !error && filtered.length === 0 && (
        <div className="model-picker-status">No models found</div>
      )}
      {!loading && !error && filtered.length > 0 && (
        <div className="model-picker-list">
          {filtered.map((m) => (
            <button
              key={m.id}
              className="model-picker-item"
              onClick={() => { onSelect(m.id); onClose(); }}
              type="button"
            >
              <span className="model-picker-item-id">{m.id}</span>
              {m.display_name && m.display_name !== m.id && (
                <span className="model-picker-item-name">{m.display_name}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProviderTabPanel -- flat form for a single provider (shown in tab view)
// ---------------------------------------------------------------------------

function ProviderTabPanel({
  provider,
  index,
  onChange,
  onDuplicate,
}: {
  provider: ProviderFormData;
  index: number;
  onChange: (index: number, updated: ProviderFormData) => void;
  onDuplicate: (index: number) => void;
}) {
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  // Model discovery state
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [modelList, setModelList] = useState<ModelItem[]>([]);
  const [modelLoading, setModelLoading] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  const update = (patch: Partial<ProviderFormData>) => {
    onChange(index, { ...provider, ...patch });
  };

  // Clear test result after 8 seconds.
  useEffect(() => {
    if (!testResult) return;
    const t = setTimeout(() => setTestResult(null), 8000);
    return () => clearTimeout(t);
  }, [testResult]);

  // Also clear test result and model picker when provider changes.
  useEffect(() => {
    setTestResult(null);
    setModelPickerOpen(false);
  }, [provider.id]);

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const msg = await invoke<string>('provider_test', {
        providerType: provider.provider_type,
        model: provider.model,
        apiKey: provider.api_key ?? '',
        apiKeyEnv: provider.api_key_env ?? '',
        baseUrl: provider.base_url ?? null,
      });
      setTestResult({ ok: true, message: msg });
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  };

  // Determine whether model discovery is available.
  const supportsDiscoveryProvider =
    provider.provider_type === 'openai-compat' ||
    provider.provider_type === 'anthropic';

  const canDiscoverModels = supportsDiscoveryProvider && !!provider.base_url?.trim();

  const handleModelSearch = async () => {
    if (!provider.base_url) return;
    setModelPickerOpen(true);
    setModelLoading(true);
    setModelError(null);
    setModelList([]);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const result = await invoke<any>('provider_list_models', {
        baseUrl: provider.base_url,
        apiKey: provider.api_key ?? '',
        apiKeyEnv: provider.api_key_env ?? '',
      });
      // OpenAI format: { data: [{ id, display_name?, ... }] }
      const items: ModelItem[] = (result?.data ?? []).map(
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (m: any) => ({ id: m.id ?? '', display_name: m.display_name ?? m.id ?? '' }),
      );
      // Sort alphabetically by id.
      items.sort((a, b) => a.id.localeCompare(b.id));
      setModelList(items);
    } catch (e) {
      setModelError(String(e));
    } finally {
      setModelLoading(false);
    }
  };

  return (
    <div className="sidetab-tab-form">
      {/* Row 0: Icon picker */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Provider</label>
          <ProviderIconPicker
            value={provider.icon}
            onChange={(icon) => update({ icon })}
          />
        </div>
      </div>

      {/* Row 1: ID + Type */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">ID</label>
          <input
            className="pf-input"
            value={provider.id}
            onChange={(e) => update({ id: e.target.value })}
            placeholder="e.g. openai-gpt4"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Provider Type</label>
          <Select
            value={provider.provider_type}
            onValueChange={(val) => update({ provider_type: val })}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select type" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="openai">OpenAI (native API)</SelectItem>
              <SelectItem value="openai-compat">OpenAI-compatible (vLLM, LiteLLM...)</SelectItem>
              <SelectItem value="anthropic">Anthropic</SelectItem>
              <SelectItem value="gemini">Gemini</SelectItem>
              <SelectItem value="deepseek">DeepSeek</SelectItem>
              <SelectItem value="ollama">Ollama</SelectItem>
              <SelectItem value="azure">Azure OpenAI</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Row 2: Model + Base URL */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Model ID</label>
          <div className="pf-model-group">
            <input
              className="pf-input"
              value={provider.model}
              onChange={(e) => update({ model: e.target.value })}
              placeholder="e.g. gpt-4o"
            />
            {canDiscoverModels && (
              <button
                className="pf-model-search-btn"
                onClick={handleModelSearch}
                title="Discover models from endpoint"
                type="button"
              >
                <Search size={13} />
              </button>
            )}
            {modelPickerOpen && (
              <ModelPickerDropdown
                models={modelList}
                loading={modelLoading}
                error={modelError}
                onSelect={(id) => update({ model: id })}
                onClose={() => setModelPickerOpen(false)}
              />
            )}
          </div>
        </div>
        <div className="pf-field">
          <label className="pf-label">Base URL</label>
          <input
            className="pf-input"
            value={provider.base_url ?? ''}
            onChange={(e) => update({ base_url: e.target.value || null })}
            placeholder={
              {
                openai: 'https://api.openai.com/v1',
                'openai-compat': 'e.g. https://api.newapi.ai/v1',
                anthropic: 'https://api.anthropic.com/v1',
                gemini: 'https://generativelanguage.googleapis.com/v1beta',
                deepseek: 'https://api.deepseek.com/v1',
                ollama: 'http://localhost:11434/v1',
                azure: 'https://RESOURCE.openai.azure.com/openai/deployments/DEPLOY',
              }[provider.provider_type] ?? 'Default'
            }
          />
        </div>
      </div>

      {/* Row 3: API Key + API Key Env */}
      <div className="pf-row">
        <div className="pf-field pf-field-key">
          <label className="pf-label">API Key</label>
          <div className="pf-key-group">
            <input
              className="pf-input"
              type={showKey ? 'text' : 'password'}
              value={provider.api_key ?? ''}
              onChange={(e) => update({ api_key: e.target.value || null })}
              placeholder="Direct key (optional)"
            />
            <button
              className="pf-key-toggle"
              onClick={() => setShowKey(!showKey)}
              title={showKey ? 'Hide' : 'Reveal'}
              type="button"
            >
              {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
        </div>
        <div className="pf-field">
          <label className="pf-label">API Key Env Var</label>
          <input
            className="pf-input"
            value={provider.api_key_env ?? ''}
            onChange={(e) => update({ api_key_env: e.target.value || null })}
            placeholder="e.g. OPENAI_API_KEY"
          />
        </div>
      </div>

      {/* Row 4: Tags -- chip editor */}
      <div className="pf-row">
        <div className="pf-field pf-field-full">
          <label className="pf-label">Tags</label>
          <TagChipInput
            tags={provider.tags}
            onChange={(next) => update({ tags: next })}
          />
          <span className="pf-hint">Routing tags -- press Enter or comma to confirm each tag</span>
        </div>
      </div>

      {/* Row 5: Numeric settings */}
      <div className="pf-row pf-row-quad">
        <div className="pf-field">
          <label className="pf-label">Max Concurrency</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            max={2}
            value={provider.max_concurrency}
            onChange={(e) => update({ max_concurrency: Number(e.target.value) || 1 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Context Window</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            min={1}
            max={2}
            value={provider.context_window}
            onChange={(e) => update({ context_window: Number(e.target.value) || 128000 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Temperature</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.1}
            min={0}
            max={2}
            value={provider.temperature ?? ''}
            onChange={(e) => update({ temperature: e.target.value ? Number(e.target.value) : null })}
            placeholder="--"
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Top P</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.05}
            min={0}
            max={1}
            value={provider.top_p ?? ''}
            onChange={(e) => update({ top_p: e.target.value ? Number(e.target.value) : null })}
            placeholder="--"
          />
        </div>
      </div>

      {/* Row 6: Cost */}
      <div className="pf-row">
        <div className="pf-field">
          <label className="pf-label">Cost / 1k Input Tokens</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.0001}
            min={0}
            value={provider.cost_per_1k_input}
            onChange={(e) => update({ cost_per_1k_input: Number(e.target.value) || 0 })}
          />
        </div>
        <div className="pf-field">
          <label className="pf-label">Cost / 1k Output Tokens</label>
          <input
            className="pf-input pf-input-num"
            type="number"
            step={0.0001}
            min={0}
            value={provider.cost_per_1k_output}
            onChange={(e) => update({ cost_per_1k_output: Number(e.target.value) || 0 })}
          />
        </div>
      </div>

      {/* Test connection row */}
      <div className="pf-action-row">
          <button
            type="button"
            className="btn-test"
            onClick={() => onDuplicate(index)}
            title="Duplicate this provider"
          >
            <Copy size={13} />
            Duplicate
          </button>
          <button
            type="button"
            className="btn-test"
            onClick={handleTest}
            disabled={testing}
          >
            {testing ? <span className="pf-spinner" /> : null}
            {testing ? 'Testing...' : 'Test Connection'}
          </button>
          {testResult && (
            <span className={`pf-test-result ${testResult.ok ? 'ok' : 'error'}`}>
              {testResult.message}
            </span>
          )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProvidersTab -- the full providers tab with sidebar list + detail panel
// ---------------------------------------------------------------------------

interface ProvidersTabProps {
  loadSection: (section: string) => Promise<string>;
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
  // Expose state upward for unified save.
  providersList: ProviderFormData[];
  setProvidersList: React.Dispatch<React.SetStateAction<ProviderFormData[]>>;
  providersMeta: string;
  setProvidersMeta: React.Dispatch<React.SetStateAction<string>>;
  setDirtyProviders: React.Dispatch<React.SetStateAction<boolean>>;
  rawProvidersToml: string | undefined;
  setRawProvidersToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function ProvidersTab({
  loadSection,
  setToast,
  providersList,
  setProvidersList,
  providersMeta,
  setProvidersMeta,
  setDirtyProviders,
  setRawProvidersToml,
}: ProvidersTabProps) {
  const [providersLoading, setProvidersLoading] = useState(false);
  const [activeProviderTab, setActiveProviderTab] = useState(0);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');

  const loadProviders = useCallback(async () => {
    setProvidersLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      setProvidersList(jsonToProviders(allConfig));

      // Also read raw TOML to cache pool-level meta fields (lines before the
      // first [[providers]] table, e.g. default_freeze_duration_secs).
      try {
        const raw: string = await loadSection('providers');
        const tableMatch = raw.match(/^\s*\[\[providers\]\]/m);
        const firstTable = tableMatch?.index ?? -1;
        setProvidersMeta(firstTable > 0 ? raw.slice(0, firstTable) : '');
      } catch {
        setProvidersMeta('');
      }
    } catch (e) {
      setToast({ message: `Failed to load providers: ${e}`, type: 'error' });
    } finally {
      setProvidersLoading(false);
    }
  }, [loadSection, setProvidersList, setProvidersMeta, setToast]);

  useEffect(() => {
    loadProviders();
  }, [loadProviders]);

  const handleProviderChange = useCallback((index: number, updated: ProviderFormData) => {
    setProvidersList((prev) => prev.map((p, i) => (i === index ? updated : p)));
    setDirtyProviders(true);
  }, [setProvidersList, setDirtyProviders]);

  const handleProviderRemove = useCallback((index: number) => {
    setProvidersList((prev) => {
      const next = prev.filter((_, i) => i !== index);
      return next;
    });
    setActiveProviderTab((prev) => Math.max(0, prev > index ? prev - 1 : Math.min(prev, providersList.length - 2)));
    setDirtyProviders(true);
  }, [providersList.length, setProvidersList, setDirtyProviders]);

  const handleProviderAdd = useCallback(() => {
    setProvidersList((prev) => {
      setActiveProviderTab(prev.length);
      return [...prev, emptyProvider()];
    });
    setDirtyProviders(true);
  }, [setProvidersList, setDirtyProviders]);

  const handleProviderDuplicate = useCallback((index: number) => {
    setProvidersList((prev) => {
      const source = prev[index];
      if (!source) return prev;
      const duplicated = { ...source, id: `${source.id}-copy` };
      const next = [...prev];
      next.splice(index + 1, 0, duplicated);
      return next;
    });
    setActiveProviderTab(index + 1);
    setDirtyProviders(true);
  }, [setProvidersList, setDirtyProviders]);

  const handleProviderMoveUp = useCallback(() => {
    if (activeProviderTab <= 0) return;
    setProvidersList((prev) => {
      const next = [...prev];
      [next[activeProviderTab - 1], next[activeProviderTab]] =
        [next[activeProviderTab], next[activeProviderTab - 1]];
      return next;
    });
    setActiveProviderTab((prev) => prev - 1);
    setDirtyProviders(true);
  }, [activeProviderTab, setProvidersList, setDirtyProviders]);

  const handleProviderMoveDown = useCallback(() => {
    if (activeProviderTab >= providersList.length - 1) return;
    setProvidersList((prev) => {
      const next = [...prev];
      [next[activeProviderTab], next[activeProviderTab + 1]] =
        [next[activeProviderTab + 1], next[activeProviderTab]];
      return next;
    });
    setActiveProviderTab((prev) => prev + 1);
    setDirtyProviders(true);
  }, [activeProviderTab, providersList.length, setProvidersList, setDirtyProviders]);

  if (providersLoading) {
    return <div className="section-loading">Loading...</div>;
  }

  const handleToggleRaw = (next: boolean) => {
    if (next) {
      // Serialize current providers list to TOML for raw editing
      const body = providersToToml(providersList);
      setRawContent(providersMeta ? `${providersMeta}${body}` : body);
    }
    setRawMode(next);
  };

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawProvidersToml(val);
            setDirtyProviders(true);
          }}
          placeholder="No providers.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
    <div className="settings-header">
      <span className="settings-header-with-toggle"><RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
    </div>
    <div className="sub-list-layout">
      {/* Left sidebar list */}
      <div className="sub-list-sidebar">
        <div className="sub-list-items">
          {providersList.map((p, i) => (
            <button
              key={i}
              className={`sub-list-item ${activeProviderTab === i ? 'active' : ''}`}
              onClick={() => setActiveProviderTab(i)}
            >
              {p.icon ? (
                <ProviderIconImg iconId={p.icon} size={16} className="sub-list-item-icon" />
              ) : (
                <Bot size={14} className="sub-list-item-icon sub-list-item-icon--default" />
              )}
              <span className="sub-list-item-label">{p.id || `Provider ${i + 1}`}</span>
              <span
                className="sub-list-item-close"
                role="button"
                tabIndex={0}
                title="Remove provider"
                onClick={(e) => { e.stopPropagation(); handleProviderRemove(i); }}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleProviderRemove(i); } }}
              >
                <X size={11} />
              </span>
            </button>
          ))}
        </div>
        <div className="sub-list-actions">
          <button
            className="sub-list-item sub-list-item-add"
            onClick={handleProviderAdd}
            title="Add provider"
          >
            <Plus size={13} />
            <span>Add</span>
          </button>
          <button
            className="sub-list-action-btn"
            onClick={handleProviderMoveUp}
            disabled={activeProviderTab <= 0 || providersList.length === 0}
            title="Move up"
          >
            <ChevronUp size={14} />
          </button>
          <button
            className="sub-list-action-btn"
            onClick={handleProviderMoveDown}
            disabled={activeProviderTab >= providersList.length - 1 || providersList.length === 0}
            title="Move down"
          >
            <ChevronDown size={14} />
          </button>
        </div>
      </div>

      {/* Right detail panel */}
      <div className="sub-list-detail">
        {providersList.length === 0 ? (
          <div className="settings-empty">
            No providers configured. Click + to add one.
          </div>
        ) : (
          <ProviderTabPanel
            key={activeProviderTab}
            provider={providersList[activeProviderTab] ?? providersList[0]}
            index={activeProviderTab < providersList.length ? activeProviderTab : 0}
            onChange={handleProviderChange}
            onDuplicate={handleProviderDuplicate}
          />
        )}
      </div>
    </div>
    </>
  );
}
