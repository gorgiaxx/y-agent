import { Save, Settings, Wrench, Sparkles, BookOpen, MessageSquare, Cpu, Gauge } from 'lucide-react';
import type { ReactElement } from 'react';
import { Button } from '../../ui/Button';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogClose } from '../../ui/Dialog';
import { ScrollArea } from '../../ui/ScrollArea';
import { MonacoEditor } from '../../ui/MonacoEditor';
import type { AgentInfo, AgentToolInfo, PromptSectionInfo } from '../../../hooks/useAgents';
import type { AgentDraft, EditorTab, EditorSurface } from '../types';
import { EDITOR_TABS } from '../types';
import { GeneralTab } from './GeneralTab';
import { ToolsTab } from './ToolsTab';
import { SkillsTab } from './SkillsTab';
import { KnowledgeTab } from './KnowledgeTab';
import { PromptTab } from './PromptTab';
import { ModelTab } from './ModelTab';
import { LimitsTab } from './LimitsTab';
import '../../settings/ProvidersTab.css';
import './AgentEditorDialog.css';

interface AgentEditorDialogProps {
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
  providerOptions: Array<{ id: string; model: string }>;
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
  onTabChange: (tab: EditorTab) => void;
  onSurfaceChange: (surface: EditorSurface) => void;
  onRawTomlChange: (content: string) => void;
  onApplyTemplate: (id: string) => void;
  onClose: () => void;
  onSave: () => void;
  onReset: () => void;
}

const TAB_ICON_MAP: Record<EditorTab, (props: { size: number; className: string }) => ReactElement> = {
  general:   (p) => <Settings {...p} />,
  tools:     (p) => <Wrench {...p} />,
  skills:    (p) => <Sparkles {...p} />,
  knowledge: (p) => <BookOpen {...p} />,
  prompt:    (p) => <MessageSquare {...p} />,
  model:     (p) => <Cpu {...p} />,
  limits:    (p) => <Gauge {...p} />,
};

export function AgentEditorDialog({
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
  providerOptions,
  onChange,
  onTabChange,
  onSurfaceChange,
  onRawTomlChange,
  onApplyTemplate,
  onClose,
  onSave,
  onReset,
}: AgentEditorDialogProps) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        size="xl"
        className="p-0 flex flex-col items-stretch text-left gap-0 h-[85vh] overflow-hidden"
      >
        {/* Header */}
        <div className="agent-editor-header">
          <div className="agent-editor-header-info">
            <DialogTitle className="text-14px">
              {mode === 'create' ? 'Create Agent' : `Edit ${draft.name || draft.id}`}
            </DialogTitle>
            <DialogDescription className="text-11px mt-0.5">
              Preset settings for reusable agent sessions.
            </DialogDescription>
          </div>
          <div className="agent-editor-header-actions">
            {canReset && (
              <Button variant="ghost" size="sm" onClick={onReset}>
                Reset
              </Button>
            )}
            <DialogClose asChild>
              <Button variant="ghost" size="sm">
                Cancel
              </Button>
            </DialogClose>
            <Button variant="primary" size="sm" onClick={onSave} disabled={saving}>
              <Save size={12} />
              {saving ? 'Saving' : 'Save'}
            </Button>
          </div>
        </div>

        {/* Body: sidebar + content */}
        <div className="agent-editor-body">
          {/* Left sidebar */}
          <div className="agent-editor-sidebar">
            <nav className="agent-editor-nav">
              {EDITOR_TABS.map((item) => {
                const isActive = tab === item.id && surface === 'form';
                const IconFn = TAB_ICON_MAP[item.id];
                const icon = IconFn
                  ? IconFn({ size: 14, className: 'agent-editor-nav-icon' })
                  : null;
                return (
                  <div
                    key={item.id}
                    className={[
                      'agent-editor-nav-item',
                      isActive ? 'agent-editor-nav-item--active' : '',
                    ].join(' ')}
                    onClick={() => {
                      if (surface === 'raw') {
                        void onSurfaceChange('form');
                      }
                      onTabChange(item.id);
                    }}
                  >
                    {icon}
                    <span className="agent-editor-nav-label">
                      {item.label}
                    </span>
                  </div>
                );
              })}
            </nav>

            {/* Raw TOML toggle (unified with Settings raw-mode-switch) */}
            <div className="agent-editor-toml-toggle">
              <label className="raw-mode-switch" title={surface === 'raw' ? 'Switch to Form view' : 'Switch to Raw TOML view'}>
                <span className={`raw-mode-switch-label ${surface === 'raw' ? '' : 'raw-mode-switch-label--active'}`}>Form</span>
                <button
                  type="button"
                  className={`raw-mode-switch-track ${surface === 'raw' ? 'raw-mode-switch-track--on' : ''}`}
                  onClick={() => void onSurfaceChange(surface === 'raw' ? 'form' : 'raw')}
                  aria-label="Toggle RAW mode"
                >
                  <span className="raw-mode-switch-thumb" />
                </button>
                <span className={`raw-mode-switch-label ${surface === 'raw' ? 'raw-mode-switch-label--active' : ''}`}>RAW</span>
              </label>
            </div>
          </div>

          {/* Right content area */}
          <ScrollArea className="flex-1 min-h-0">
            <div className="agent-editor-content">
              {rawError && (
                <div className="agent-editor-error">
                  {rawError}
                </div>
              )}

              {surface === 'raw' ? (
                <div className="agent-editor-raw">
                  <div className="agent-editor-raw-info">
                    <span className="agent-editor-raw-title">
                      {rawUsesSourceFile ? 'Editing source file' : 'Editing generated agent source'}
                    </span>
                    <span className="agent-editor-raw-path">
                      {rawPath ?? 'The content below will be saved as the agent definition TOML.'}
                    </span>
                  </div>
                  <MonacoEditor
                    value={rawToml}
                    onChange={(val) => onRawTomlChange(val)}
                    language="toml"
                    height="400px"
                  />
                </div>
              ) : (
                <>
                  {tab === 'general' && (
                    <GeneralTab
                      mode={mode}
                      draft={draft}
                      agents={agents}
                      onChange={onChange}
                      onApplyTemplate={onApplyTemplate}
                    />
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
                </>
              )}
            </div>
          </ScrollArea>
        </div>
      </DialogContent>
    </Dialog>
  );
}
