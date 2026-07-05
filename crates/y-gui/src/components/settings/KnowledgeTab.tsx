// ---------------------------------------------------------------------------
// KnowledgeTab -- Knowledge Base configuration form
// ---------------------------------------------------------------------------

import { useState } from 'react';
import type { KnowledgeFormData } from './settingsTypes';
import { jsonToKnowledge } from './settingsTypes';
import { KNOWLEDGE_SCHEMA } from '../../utils/settingsSchemas';
import { SettingsTabShell } from './SettingsTabShell';
import { useSettingsTab } from './useSettingsTab';
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
  const [showApiKey, setShowApiKey] = useState(false);

  const { loading, rawMode, rawContent, handleToggleRaw, handleRawChange, update } = useSettingsTab({
    section: 'knowledge',
    schema: KNOWLEDGE_SCHEMA,
    configKey: 'knowledge',
    form: knowledgeForm,
    setForm: setKnowledgeForm,
    setDirty: setDirtyKnowledge,
    setRawToml: setRawKnowledgeToml,
    jsonToForm: jsonToKnowledge,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={loading}
      rawMode={rawMode}
      rawContent={rawContent}
      onToggleRaw={handleToggleRaw}
      onRawChange={handleRawChange}
      rawPlaceholder="No knowledge.toml found. Content will be created on save."
      form={
        <div className="settings-form-wrap">
          <SettingsGroup title="Chunking">
            <SettingsItem title="L0 Max Tokens" description="Summary level">
              <Input
                numeric type="number" min={50} className="w-[100px]"
                value={knowledgeForm.l0_max_tokens}
                onChange={(e) => update({ l0_max_tokens: Number(e.target.value) || 200 })}
              />
            </SettingsItem>
            <SettingsItem title="L1 Max Tokens" description="Section overview">
              <Input
                numeric type="number" min={100} className="w-[100px]"
                value={knowledgeForm.l1_max_tokens}
                onChange={(e) => update({ l1_max_tokens: Number(e.target.value) || 500 })}
              />
            </SettingsItem>
            <SettingsItem title="L2 Max Tokens" description="Paragraph level">
              <Input
                numeric type="number" min={100} className="w-[100px]"
                value={knowledgeForm.l2_max_tokens}
                onChange={(e) => update({ l2_max_tokens: Number(e.target.value) || 500 })}
              />
            </SettingsItem>
            <SettingsItem title="Max Chunks / Entry">
              <Input
                numeric type="number" min={100} step={500} className="w-[100px]"
                value={knowledgeForm.max_chunks_per_entry}
                onChange={(e) => update({ max_chunks_per_entry: Number(e.target.value) || 5000 })}
              />
            </SettingsItem>
            <SettingsItem title="Default Collection" wide>
              <Input
                value={knowledgeForm.default_collection}
                onChange={(e) => update({ default_collection: e.target.value })}
                placeholder="default"
              />
            </SettingsItem>
            <SettingsItem title="Min Similarity Threshold" description="Results below are discarded (0.0-1.0).">
              <Input
                numeric type="number" min={0} max={1} step={0.05} className="w-[100px]"
                value={knowledgeForm.min_similarity_threshold}
                onChange={(e) => update({ min_similarity_threshold: Number(e.target.value) || 0.65 })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="Embedding">
            <SettingsItem title="Enable Embedding" description="Requires an OpenAI-compatible embedding API.">
              <Checkbox
                checked={knowledgeForm.embedding_enabled}
                onCheckedChange={(c) => update({ embedding_enabled: c === true })}
              />
            </SettingsItem>
            <SettingsItem title="Embedding Model" wide>
              <Input
                value={knowledgeForm.embedding_model}
                onChange={(e) => update({ embedding_model: e.target.value })}
                placeholder="text-embedding-3-small"
              />
            </SettingsItem>
            <SettingsItem title="Dimensions">
              <Input
                numeric type="number" min={64} className="w-[100px]"
                value={knowledgeForm.embedding_dimensions}
                onChange={(e) => update({ embedding_dimensions: Number(e.target.value) || 1536 })}
              />
            </SettingsItem>
            <SettingsItem title="Embedding Base URL" wide>
              <Input
                value={knowledgeForm.embedding_base_url}
                onChange={(e) => update({ embedding_base_url: e.target.value })}
                placeholder="https://api.openai.com/v1"
              />
            </SettingsItem>
            <SettingsItem title="API Key Env Variable" wide>
              <Input
                value={knowledgeForm.embedding_api_key_env}
                onChange={(e) => update({ embedding_api_key_env: e.target.value })}
                placeholder="OPENAI_API_KEY"
              />
            </SettingsItem>
            <SettingsItem title="API Key (direct)" description="Takes precedence over env var." wide>
              <div className="pf-key-group w-full">
                <Input
                  className="flex-1 min-w-0 pr-[30px]"
                  type={showApiKey ? 'text' : 'password'}
                  value={knowledgeForm.embedding_api_key}
                  onChange={(e) => update({ embedding_api_key: e.target.value })}
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
                onChange={(e) => update({ embedding_max_tokens: Number(e.target.value) || 0 })}
              />
            </SettingsItem>
          </SettingsGroup>

          <SettingsGroup title="Retrieval Tuning">
            <SettingsItem title="Strategy">
              <Select
                value={knowledgeForm.retrieval_strategy}
                onValueChange={(val) => update({ retrieval_strategy: val })}
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
                onChange={(e) => update({ bm25_weight: Number(e.target.value) || 1.0 })}
              />
            </SettingsItem>
            <SettingsItem title="Vector Weight">
              <Input
                numeric type="number" min={0} step={0.1} className="w-[100px]"
                value={knowledgeForm.vector_weight}
                onChange={(e) => update({ vector_weight: Number(e.target.value) || 1.0 })}
              />
            </SettingsItem>
          </SettingsGroup>
        </div>
      }
    />
  );
}
