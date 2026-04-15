import { Save, FileCode2, Settings, Wrench, Sparkles, BookOpen, MessageSquare, Cpu, Gauge } from 'lucide-react';
import type { ReactElement } from 'react';
import { Button } from '../../ui/Button';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogClose } from '../../ui/Dialog';
import { ScrollArea } from '../../ui/ScrollArea';
import { Switch } from '../../ui/Switch';
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
        width="720px"
        className="p-0 flex flex-col items-stretch text-left gap-0 max-h-[90vh] overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-center justify-between gap-4 px-4 py-3">
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

        {/* Body: sidebar + content */}
        <div className="flex flex-1 min-h-0">
          {/* Left sidebar */}
          <div className="flex flex-col w-44 flex-shrink-0">
            <nav className="flex flex-col py-1 px-2 gap-0.5 flex-1">
              {EDITOR_TABS.map((item) => {
                const isActive = tab === item.id && surface === 'form';
                const IconFn = TAB_ICON_MAP[item.id];
                const icon = IconFn
                  ? IconFn({ size: 14, className: `shrink-0 ${isActive ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]'}` })
                  : null;
                return (
                  <div
                    key={item.id}
                    className={[
                      'flex items-center gap-1.5',
                      'px-2 py-2.5',
                      'rounded-[var(--radius-md)]',
                      'cursor-pointer',
                      'transition-colors duration-120',
                      'mb-0.5',
                      'border border-solid',
                      isActive
                        ? 'bg-[rgba(255,255,255,0.06)] border-[rgba(255,255,255,0.06)]'
                        : 'border-transparent hover:bg-[var(--surface-hover)]',
                    ].join(' ')}
                    onClick={() => {
                      if (surface === 'raw') {
                        void onSurfaceChange('form');
                      }
                      onTabChange(item.id);
                    }}
                  >
                    {icon}
                    <span className={[
                      'text-13px font-600',
                      'flex-1 min-w-0',
                      'whitespace-nowrap overflow-hidden text-ellipsis',
                      isActive ? 'text-[var(--text-primary)]' : 'text-[var(--text-primary)]',
                    ].join(' ')}>
                      {item.label}
                    </span>
                  </div>
                );
              })}
            </nav>

            {/* Raw TOML toggle */}
            <div className="px-3 py-3">
              <label className="flex items-center justify-between gap-2 cursor-pointer">
                <span className="text-11px text-[var(--text-muted)] flex items-center gap-1.5">
                  <FileCode2 size={12} />
                  Raw TOML
                </span>
                <Switch
                  checked={surface === 'raw'}
                  onCheckedChange={(checked) => void onSurfaceChange(checked ? 'raw' : 'form')}
                />
              </label>
            </div>
          </div>

          {/* Right content area */}
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
        </div>
      </DialogContent>
    </Dialog>
  );
}
