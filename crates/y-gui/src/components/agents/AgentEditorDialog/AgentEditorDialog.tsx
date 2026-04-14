import { Save, FileCode2, SlidersHorizontal } from 'lucide-react';
import { Button } from '../../ui/Button';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogClose } from '../../ui/Dialog';
import { ScrollArea } from '../../ui/ScrollArea';
import { Separator } from '../../ui/Separator';
import { Textarea } from '../../ui/Input';
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
        width="680px"
        className="p-0 flex flex-col items-stretch text-left gap-0 max-h-[90vh] overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-start justify-between gap-4 p-4 border-b border-solid border-[var(--border)]">
          <div className="min-w-0">
            <DialogTitle className="text-14px">
              {mode === 'create' ? 'Create Agent' : `Edit ${draft.name || draft.id}`}
            </DialogTitle>
            <DialogDescription className="text-11px mt-0.5">
              Preset settings for reusable agent sessions.
            </DialogDescription>
          </div>
          <div className="flex items-center gap-1.5 flex-shrink-0">
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

        {/* Surface switch + Tabs */}
        <div className="flex items-center gap-2 px-4 py-2 border-b border-solid border-[var(--border)] bg-[var(--surface-secondary)]">
          <div className="flex items-center gap-1 p-0.5 rounded-[var(--radius-md)] bg-[var(--surface-tertiary)]">
            <button
              className={[
                'flex items-center gap-1 px-2 py-1 text-11px font-500 rounded-[var(--radius-sm)]',
                'transition-colors duration-150',
                surface === 'form'
                  ? 'bg-[var(--surface-primary)] text-[var(--text-primary)] shadow-sm'
                  : 'text-[var(--text-muted)] hover:text-[var(--text-secondary)]',
              ].join(' ')}
              onClick={() => onSurfaceChange('form')}
            >
              <SlidersHorizontal size={12} />
              Form
            </button>
            <button
              className={[
                'flex items-center gap-1 px-2 py-1 text-11px font-500 rounded-[var(--radius-sm)]',
                'transition-colors duration-150',
                surface === 'raw'
                  ? 'bg-[var(--surface-primary)] text-[var(--text-primary)] shadow-sm'
                  : 'text-[var(--text-muted)] hover:text-[var(--text-secondary)]',
              ].join(' ')}
              onClick={() => onSurfaceChange('raw')}
            >
              <FileCode2 size={12} />
              Raw TOML
            </button>
          </div>

          {surface === 'form' && (
            <>
              <Separator orientation="vertical" className="h-4 mx-1" />
              <div className="flex items-center gap-0.5 overflow-x-auto">
                {EDITOR_TABS.map((item) => (
                  <button
                    key={item.id}
                    className={[
                      'px-2 py-1 text-11px font-500 rounded-[var(--radius-sm)]',
                      'transition-colors duration-150 whitespace-nowrap',
                      tab === item.id
                        ? 'bg-[var(--accent-subtle)] text-[var(--accent)]'
                        : 'text-[var(--text-muted)] hover:text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]',
                    ].join(' ')}
                    onClick={() => onTabChange(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>

        {/* Body */}
        <ScrollArea className="flex-1 min-h-0">
          <div className="p-4">
            {rawError && (
              <div className="mb-3 px-3 py-2 text-11px rounded-[var(--radius-md)] bg-[var(--error-subtle)] text-[var(--error)] border border-solid border-[rgba(229,115,115,0.2)]">
                {rawError}
              </div>
            )}

            {surface === 'raw' ? (
              <div className="flex flex-col gap-3">
                <div className="flex flex-col gap-1">
                  <span className="text-11px font-500 text-[var(--text-primary)]">
                    {rawUsesSourceFile ? 'Editing source file' : 'Editing generated agent source'}
                  </span>
                  <span className="text-10px text-[var(--text-muted)] break-all">
                    {rawPath ?? 'The content below will be saved as the agent definition TOML.'}
                  </span>
                </div>
                <Textarea
                  variant="mono"
                  value={rawToml}
                  onChange={(event) => onRawTomlChange(event.target.value)}
                  rows={20}
                  className="min-h-[400px] text-11px"
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
      </DialogContent>
    </Dialog>
  );
}
