// ---------------------------------------------------------------------------
// KnowledgeTab -- Knowledge Base configuration form
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback, useRef } from 'react';
import { transport } from '../../lib';
import type { KnowledgeFormData } from './settingsTypes';
import { jsonToKnowledge } from './settingsTypes';
import { RawTomlEditor, RawModeToggle } from './TomlEditorTab';
import { mergeIntoRawToml } from '../../utils/tomlUtils';
import { KNOWLEDGE_SCHEMA } from '../../utils/settingsSchemas';
import { Eye, EyeOff } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/Select';
import { Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';

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
      const allConfig = await transport.invoke<any>('config_get');
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
          <h3 className="section-title section-title--flush">
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
        <h3 className="section-title section-title--flush">
          <span className="settings-header-with-toggle">Knowledge <RawModeToggle rawMode={rawMode} onToggle={handleToggleRaw} /></span>
        </h3>
      </div>
      <div className="settings-form-wrap">
        <SettingsGroup title="Chunking">
          <SettingsItem title="L0 Max Tokens" description="Summary level">
            <Input
              numeric type="number" min={50} className="w-[100px]"
              value={knowledgeForm.l0_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l0_max_tokens: Number(e.target.value) || 200 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="L1 Max Tokens" description="Section overview">
            <Input
              numeric type="number" min={100} className="w-[100px]"
              value={knowledgeForm.l1_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l1_max_tokens: Number(e.target.value) || 500 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="L2 Max Tokens" description="Paragraph level">
            <Input
              numeric type="number" min={100} className="w-[100px]"
              value={knowledgeForm.l2_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, l2_max_tokens: Number(e.target.value) || 500 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Max Chunks / Entry">
            <Input
              numeric type="number" min={100} step={500} className="w-[100px]"
              value={knowledgeForm.max_chunks_per_entry}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, max_chunks_per_entry: Number(e.target.value) || 5000 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Default Collection" wide>
            <Input
              value={knowledgeForm.default_collection}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, default_collection: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="default"
            />
          </SettingsItem>
          <SettingsItem title="Min Similarity Threshold" description="Results below are discarded (0.0-1.0).">
            <Input
              numeric type="number" min={0} max={1} step={0.05} className="w-[100px]"
              value={knowledgeForm.min_similarity_threshold}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, min_similarity_threshold: Number(e.target.value) || 0.65 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Embedding">
          <SettingsItem title="Enable Embedding" description="Requires an OpenAI-compatible embedding API.">
            <Checkbox
              checked={knowledgeForm.embedding_enabled}
              onCheckedChange={(c) => { setKnowledgeForm({ ...knowledgeForm, embedding_enabled: c === true }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Embedding Model" wide>
            <Input
              value={knowledgeForm.embedding_model}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_model: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="text-embedding-3-small"
            />
          </SettingsItem>
          <SettingsItem title="Dimensions">
            <Input
              numeric type="number" min={64} className="w-[100px]"
              value={knowledgeForm.embedding_dimensions}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_dimensions: Number(e.target.value) || 1536 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Embedding Base URL" wide>
            <Input
              value={knowledgeForm.embedding_base_url}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_base_url: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="https://api.openai.com/v1"
            />
          </SettingsItem>
          <SettingsItem title="API Key Env Variable" wide>
            <Input
              value={knowledgeForm.embedding_api_key_env}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_api_key_env: e.target.value }); setDirtyKnowledge(true); }}
              placeholder="OPENAI_API_KEY"
            />
          </SettingsItem>
          <SettingsItem title="API Key (direct)" description="Takes precedence over env var." wide>
            <div className="pf-key-group w-full">
              <Input
                className="flex-1 min-w-0 pr-[30px]"
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
          </SettingsItem>
          <SettingsItem title="Embedding Max Tokens" description="0 = falls back to l2_max_tokens. 512 recommended for local GGUF models.">
            <Input
              numeric type="number" min={0} step={128} className="w-[100px]"
              value={knowledgeForm.embedding_max_tokens}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, embedding_max_tokens: Number(e.target.value) || 0 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
        </SettingsGroup>

        <SettingsGroup title="Retrieval Tuning">
          <SettingsItem title="Strategy">
            <Select
              value={knowledgeForm.retrieval_strategy}
              onValueChange={(val) => { setKnowledgeForm({ ...knowledgeForm, retrieval_strategy: val }); setDirtyKnowledge(true); }}
            >
              <SelectTrigger className="w-[220px]">
                <SelectValue placeholder="Select strategy" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="hybrid">Hybrid (Vector + BM25)</SelectItem>
                <SelectItem value="keyword">Keyword (BM25 only)</SelectItem>
                <SelectItem value="semantic">Semantic (Vector only)</SelectItem>
              </SelectContent>
            </Select>
          </SettingsItem>
          <SettingsItem title="BM25 Weight">
            <Input
              numeric type="number" min={0} step={0.1} className="w-[100px]"
              value={knowledgeForm.bm25_weight}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, bm25_weight: Number(e.target.value) || 1.0 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
          <SettingsItem title="Vector Weight">
            <Input
              numeric type="number" min={0} step={0.1} className="w-[100px]"
              value={knowledgeForm.vector_weight}
              onChange={(e) => { setKnowledgeForm({ ...knowledgeForm, vector_weight: Number(e.target.value) || 1.0 }); setDirtyKnowledge(true); }}
            />
          </SettingsItem>
        </SettingsGroup>
      </div>
    </>
  );
}
