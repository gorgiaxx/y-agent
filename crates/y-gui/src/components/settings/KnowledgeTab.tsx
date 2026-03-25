// ---------------------------------------------------------------------------
// KnowledgeTab -- Knowledge Base configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { KnowledgeFormData } from './settingsTypes';
import { jsonToKnowledge } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { KNOWLEDGE_SCHEMA } from '../../utils/settingsSchemas';
import { Eye, EyeOff } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox } from '../ui';

interface KnowledgeTabProps {
  loadSection: (section: string) => Promise<string>;
  knowledgeForm: KnowledgeFormData;
  setKnowledgeForm: React.Dispatch<React.SetStateAction<KnowledgeFormData>>;
  setDirtyKnowledge: React.Dispatch<React.SetStateAction<boolean>>;
  setRawKnowledgeToml: React.Dispatch<React.SetStateAction<string | undefined>>;
}

export function KnowledgeTab({
  loadSection,
  knowledgeForm,
  setKnowledgeForm,
  setDirtyKnowledge,
  setRawKnowledgeToml,
}: KnowledgeTabProps) {
  const [loading, setLoading] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [rawContent, setRawContent] = useState('');
  const cachedRawToml = useRef<string | undefined>(undefined);
  const [showApiKey, setShowApiKey] = useState(false);

  const loadForm = useCallback(async () => {
    setLoading(true);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const allConfig = await invoke<any>('config_get');
      const json = allConfig?.knowledge ?? {};
      setKnowledgeForm(jsonToKnowledge(json));
      try {
        const raw = await loadSection('knowledge');
        setRawKnowledgeToml(raw);
        cachedRawToml.current = raw;
        setRawContent(raw);
      } catch {
        setRawKnowledgeToml(undefined);
        cachedRawToml.current = undefined;
        setRawContent('');
      }
    } catch {
      // Use defaults if not found.
    } finally {
      setLoading(false);
    }
  }, [loadSection, setKnowledgeForm, setRawKnowledgeToml]);

  useEffect(() => {
    loadForm();
  }, [loadForm]);

  const handleToggleRaw = useCallback((next: boolean) => {
    if (next) {
      setRawContent(mergeIntoRawToml(cachedRawToml.current, knowledgeForm as unknown as Record<string, unknown>, KNOWLEDGE_SCHEMA));
    }
    setRawMode(next);
  }, [knowledgeForm]);

  if (loading) {
    return <div className="section-loading">Loading...</div>;
  }

  if (rawMode) {
    return (
      <>
        <div className="settings-header">
          <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
            <span className="settings-header-with-toggle">Knowledge <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
          </h3>
        </div>
        <RawTomlEditor
          content={rawContent}
          onChange={(val) => {
            setRawContent(val);
            setRawKnowledgeToml(val);
            setDirtyKnowledge(true);
          }}
          placeholder="No knowledge.toml found. Content will be created on save."
        />
      </>
    );
  }

  return (
    <>
      <div className="settings-header">
        <h3 className="section-title" style={{ margin: 0, padding: 0, border: 'none' }}>
          <span className="settings-header-with-toggle">Knowledge <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        {/* Chunking */}
        <div className="pf-section-divider" style={{ marginTop: 0 }}>
          <span className="pf-section-title">Chunking</span>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">L0 Max Tokens</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={50}
              value={knowledgeForm.l0_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l0_max_tokens: Number(e.target.value) || 200 }); setDirtyKnowledge(true); }}
            />
            <span className="pf-hint">Summary level</span>
          </div>
          <div className="pf-field">
            <label className="pf-label">L1 Max Tokens</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={100}
              value={knowledgeForm.l1_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l1_max_tokens: Number(e.target.value) || 500 }); setDirtyKnowledge(true); }}
            />
            <span className="pf-hint">Section overview</span>
          </div>
          <div className="pf-field">
            <label className="pf-label">L2 Max Tokens</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={100}
              value={knowledgeForm.l2_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l2_max_tokens: Number(e.target.value) || 500 }); setDirtyKnowledge(true); }}
            />
            <span className="pf-hint">Paragraph level</span>
          </div>
          <div className="pf-field">
            <label className="pf-label">Max Chunks / Entry</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={100}
              step={500}
              value={knowledgeForm.max_chunks_per_entry}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, max_chunks_per_entry: Number(e.target.value) || 5000 }); setDirtyKnowledge(true); }}
            />
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field">
            <label className="pf-label">Default Collection</label>
            <input
              className="pf-input"
              value={knowledgeForm.default_collection}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, default_collection: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="default"
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Min Similarity Threshold</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={knowledgeForm.min_similarity_threshold}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, min_similarity_threshold: Number(e.target.value) || 0.65 }); setDirtyKnowledge(true); }}
            />
            <span className="pf-hint">Results below are discarded (0.0-1.0).</span>
          </div>
        </div>

        {/* Embedding */}
        <div className="pf-section-divider">
          <span className="pf-section-title">Embedding</span>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">
              <Checkbox
                checked={knowledgeForm.embedding_enabled}
                onCheckedChange={(c) => { setKnowledgeForm({ ...knowledgeForm, embedding_enabled: c === true }); setDirtyKnowledge(true); }}
              />
              {' '}Enable Embedding
            </label>
            <span className="pf-hint">Requires an OpenAI-compatible embedding API.</span>
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field" style={{ flex: 2 }}>
            <label className="pf-label">Embedding Model</label>
            <input
              className="pf-input"
              value={knowledgeForm.embedding_model}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_model: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="text-embedding-3-small"
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Dimensions</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={64}
              value={knowledgeForm.embedding_dimensions}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_dimensions: Number(e.target.value) || 1536 }); setDirtyKnowledge(true); }}
            />
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field pf-field-full">
            <label className="pf-label">Embedding Base URL</label>
            <input
              className="pf-input"
              value={knowledgeForm.embedding_base_url}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_base_url: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="https://api.openai.com/v1"
            />
          </div>
        </div>
        <div className="pf-row">
          <div className="pf-field">
            <label className="pf-label">API Key Env Variable</label>
            <input
              className="pf-input"
              value={knowledgeForm.embedding_api_key_env}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_api_key_env: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="OPENAI_API_KEY"
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">API Key (direct)</label>
            <div className="pf-key-group">
              <input
                className="pf-input"
                type={showApiKey ? 'text' : 'password'}
                value={knowledgeForm.embedding_api_key}
                onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_api_key: e.target.value }); setDirtyKnowledge(true); }}
                placeholder="sk-..."
              />
              <button
                className="pf-key-toggle"
                onClick={() => setShowApiKey(!showApiKey)}
                title={showApiKey ? 'Hide' : 'Show'}
                type="button"
              >
                {showApiKey ? <EyeOff size={13} /> : <Eye size={13} />}
              </button>
            </div>
            <span className="pf-hint">Takes precedence over env var.</span>
          </div>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">Embedding Max Tokens</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={0}
              step={128}
              value={knowledgeForm.embedding_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_max_tokens: Number(e.target.value) || 0 }); setDirtyKnowledge(true); }}
            />
            <span className="pf-hint">0 = falls back to l2_max_tokens. 512 recommended for local GGUF models.</span>
          </div>
        </div>

        {/* Retrieval */}
        <div className="pf-section-divider">
          <span className="pf-section-title">Retrieval Tuning</span>
        </div>
        <div className="pf-row pf-row-quad">
          <div className="pf-field">
            <label className="pf-label">Strategy</label>
            <Select
              value={knowledgeForm.retrieval_strategy}
              onValueChange={(val) => { setKnowledgeForm({ ...knowledgeForm, retrieval_strategy: val }); setDirtyKnowledge(true); }}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select strategy" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="hybrid">Hybrid (Vector + BM25)</SelectItem>
                <SelectItem value="keyword">Keyword (BM25 only)</SelectItem>
                <SelectItem value="semantic">Semantic (Vector only)</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="pf-field">
            <label className="pf-label">BM25 Weight</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={0}
              step={0.1}
              value={knowledgeForm.bm25_weight}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, bm25_weight: Number(e.target.value) || 1.0 }); setDirtyKnowledge(true); }}
            />
          </div>
          <div className="pf-field">
            <label className="pf-label">Vector Weight</label>
            <input
              className="pf-input pf-input-num"
              type="number"
              min={0}
              step={0.1}
              value={knowledgeForm.vector_weight}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, vector_weight: Number(e.target.value) || 1.0 }); setDirtyKnowledge(true); }}
            />
          </div>
        </div>
      </div>
    </>
  );
}
