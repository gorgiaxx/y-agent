import { Button } from '../ui/Button';
import { ScrollArea } from '../ui/ScrollArea';
import { MonacoEditor } from '../ui/MonacoEditor';
import type { AgentInfo, AgentToolInfo, PromptSectionInfo } from '../../hooks/useAgents';
import type { AgentDraft, EditorTab, EditorSurface } from '../agents/types';
import { GeneralTab } from './AgentEditorDialog/GeneralTab';
import { ToolsTab } from './AgentEditorDialog/ToolsTab';
import { SkillsTab } from './AgentEditorDialog/SkillsTab';
import { KnowledgeTab } from './AgentEditorDialog/KnowledgeTab';
import { PromptTab } from './AgentEditorDialog/PromptTab';
import { ModelTab } from './AgentEditorDialog/ModelTab';
import { LimitsTab } from './AgentEditorDialog/LimitsTab';
import { McpTab } from './AgentEditorDialog/McpTab';
import '../settings/SettingsPanel.css';
import '../settings/SettingsForm.css';
import './AgentEditorPanel.css';

interface AgentEditorPanelProps {
  mode: 'create' | 'edit';
  draft: AgentDraft;
  tab: EditorTab;
  surface: EditorSurface;
  rawToml: string;
  rawPath: string | null;
  rawUsesSourceFile: boolean;
  rawError: string | null;
  saving: boolean;
  canReset: boolean;
  agents: AgentInfo[];
  tools: AgentToolInfo[];
  promptSections: PromptSectionInfo[];
  availableSkills: string[];
  knowledgeCollections: string[];
  mcpServers: { name: string; disabled: boolean }[];
  providerOptions: Array<{ id: string; model: string }>;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
  onRawTomlChange: (content: string) => void;
  onApplyTemplate: (id: string) => void;
  onSave: () => void;
  onReset: () => void;
}

export function AgentEditorPanel({
  mode,
  draft,
  tab,
  surface,
  rawToml,
  rawPath,
  rawUsesSourceFile,
  rawError,
  saving,
  canReset,
  agents,
  tools,
  promptSections,
  availableSkills,
  knowledgeCollections,
  mcpServers,
  providerOptions,
  onChange,
  onRawTomlChange,
  onApplyTemplate,
  onSave,
  onReset,
}: AgentEditorPanelProps) {
  return (
    <div className="settings-panel">
      <div className="settings-action-bar">
        <h2 className="settings-action-bar-title">
          {mode === 'create' ? 'Create Agent' : `Edit ${draft.name || draft.id}`}
        </h2>
        <div className="settings-action-bar-actions">
          {canReset && (
            <Button variant="ghost" size="sm" onClick={onReset}>
              Reset
            </Button>
          )}
          <Button variant="primary" onClick={onSave} disabled={saving}>
            {saving ? 'Saving...' : 'Save Changes'}
          </Button>
        </div>
      </div>

      {surface === 'raw' ? (
        <div className="settings-content agent-editor-raw-surface">
          {rawError && (
            <div className="settings-toast error">{rawError}</div>
          )}
          <div style={{ fontSize: 12, color: 'var(--text-secondary)', marginBottom: 4 }}>
            {rawUsesSourceFile ? 'Editing source file' : 'Editing generated agent source'}
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 8 }}>
            {rawPath ?? 'The content below will be saved as the agent definition TOML.'}
          </div>
          <div className="toml-editor-monaco">
            <MonacoEditor
              value={rawToml}
              onChange={(val) => onRawTomlChange(val)}
              language="toml"
            />
          </div>
        </div>
      ) : (
        <ScrollArea className="flex-1 min-h-0">
          <div className="settings-content">
            {rawError && (
              <div className="settings-toast error">{rawError}</div>
            )}
            {tab === 'general' && (
              <GeneralTab mode={mode} draft={draft} agents={agents} onChange={onChange} onApplyTemplate={onApplyTemplate} />
            )}
            {tab === 'tools' && (
              <ToolsTab draft={draft} tools={tools} onChange={onChange} />
            )}
            {tab === 'skills' && (
              <SkillsTab draft={draft} availableSkills={availableSkills} onChange={onChange} />
            )}
            {tab === 'knowledge' && (
              <KnowledgeTab draft={draft} knowledgeCollections={knowledgeCollections} onChange={onChange} />
            )}
            {tab === 'prompt' && (
              <PromptTab draft={draft} promptSections={promptSections} onChange={onChange} />
            )}
            {tab === 'model' && (
              <ModelTab draft={draft} providerOptions={providerOptions} onChange={onChange} />
            )}
            {tab === 'limits' && (
              <LimitsTab draft={draft} onChange={onChange} />
            )}
            {tab === 'mcp' && (
              <McpTab draft={draft} mcpServers={mcpServers} onChange={onChange} />
            )}
          </div>
        </ScrollArea>
      )}
    </div>
  );
}
