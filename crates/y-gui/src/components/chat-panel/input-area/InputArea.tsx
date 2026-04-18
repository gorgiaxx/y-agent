import { useState, useRef, useCallback, useEffect } from 'react';
import { Square, X, AtSign, Maximize2, Minimize2, Paintbrush, Eraser, BookOpen, Bot, Lightbulb, Paperclip, Zap, ScanSearch, ClipboardList, ScrollText, Languages, Loader2, Cpu } from 'lucide-react';
import { transport, platform } from '../../../lib';
import { ProviderIconImg } from '../../common/ProviderIconPicker';
import { ConfirmDialog } from '../../common/ConfirmDialog';
import { CommandMenu } from './CommandMenu';
import { AskUserDialog } from './AskUserDialog';
import { PermissionDialog } from './PermissionDialog';
import { SessionPromptDialog } from '../SessionPromptDialog';
import { ContentEditableInput, type ContentEditableInputHandle } from './ContentEditableInput';
import { GUI_COMMANDS } from '../../../commands';
import type { GuiCommandDef } from '../../../commands';
import type { ProviderInfo, SkillInfo, KnowledgeCollectionInfo, ThinkingEffort, PlanMode, McpMode, Attachment } from '../../../types';
import type { PendingEdit } from '../../../hooks/useChat';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string, skills?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode, mcpMode?: McpMode | null, mcpServers?: string[]) => void;
  onStop?: () => void;
  onCommand?: (commandName: string) => boolean;
  disabled: boolean;
  sendOnEnter: boolean;
  providers: ProviderInfo[];
  selectedProviderId: string;
  onSelectProvider: (id: string) => void;
  pendingEdit?: PendingEdit | null;
  onCancelEdit?: () => void;
  skills?: SkillInfo[];
  knowledgeCollections?: KnowledgeCollectionInfo[];
  expanded?: boolean;
  onExpandChange?: (expanded: boolean) => void;
  onClearSession?: () => void;
  onAddContextReset?: () => void;
  /** Map from provider ID to icon identifier. */
  providerIcons?: Record<string, string>;
  /** Current thinking effort level (null = model default). */
  thinkingEffort?: ThinkingEffort | null;
  /** Callback when user changes thinking effort. */
  onThinkingEffortChange?: (effort: ThinkingEffort | null) => void;
  /** Controlled plan-mode value. When omitted, InputArea uses local state. */
  planMode?: PlanMode;
  /** Callback when user changes plan mode. Enables controlled usage. */
  onPlanModeChange?: (mode: PlanMode) => void;
  /** Persist uncontrolled plan mode to localStorage. Defaults to true. */
  persistPlanMode?: boolean;
  /** Pending AskUser interaction data. */
  askUserData?: {
    interactionId: string;
    questions: Array<{
      question: string;
      options: string[];
      multi_select?: boolean;
    }>;
  } | null;
  /** Callback when user submits answers to AskUser. */
  onAskUserSubmit?: (interactionId: string, answers: Record<string, string>) => void;
  /** Callback when user dismisses AskUser dialog. */
  onAskUserDismiss?: (interactionId: string) => void;
  /** Pending permission request data. */
  permissionData?: {
    requestId: string;
    toolName: string;
    actionDescription: string;
    reason: string;
    contentPreview?: string | null;
  } | null;
  /** Callback when user approves a permission request. */
  onPermissionApprove?: (requestId: string) => void;
  /** Callback when user denies a permission request. */
  onPermissionDeny?: (requestId: string) => void;
  /** Callback when user allows all future tool calls for this session. */
  onPermissionAllowAllForSession?: (requestId: string) => void;
  /** Whether context compaction is in progress. */
  isCompacting?: boolean;
  /** Active session ID (for per-session features like custom prompt). */
  sessionId?: string | null;
  /** Whether the active session has a custom system prompt. */
  hasCustomPrompt?: boolean;
  /** Callback when custom prompt state changes. */
  onCustomPromptChange?: (hasPrompt: boolean) => void;
  /** Draft text to populate after rewind/undo (normal draft, not edit mode). */
  rewindDraft?: string | null;
  /** Called after rewindDraft is consumed to clear the state. */
  onRewindDraftConsumed?: () => void;
  /** MCP mode for the current session. */
  mcpMode?: McpMode;
  /** Callback when MCP mode changes. */
  onMcpModeChange?: (mode: McpMode) => void;
  /** List of configured MCP servers (for manual mode selection). */
  mcpServerList?: { name: string; disabled: boolean }[];
  /** Selected MCP server names (for manual mode). */
  selectedMcpServers?: string[];
  /** Callback when manual-mode server selection changes. */
  onMcpServerToggle?: (serverName: string) => void;
}

export function InputArea({
  onSend,
  onStop,
  onCommand,
  disabled,
  sendOnEnter,
  providers,
  selectedProviderId,
  onSelectProvider,
  pendingEdit,
  onCancelEdit,
  skills = [],
  knowledgeCollections = [],
  expanded = false,
  onExpandChange,
  onClearSession,
  onAddContextReset,
  providerIcons,
  thinkingEffort,
  onThinkingEffortChange,
  planMode: controlledPlanMode,
  onPlanModeChange,
  persistPlanMode = true,
  askUserData,
  onAskUserSubmit,
  onAskUserDismiss,
  permissionData,
  onPermissionApprove,
  onPermissionDeny,
  onPermissionAllowAllForSession,
  isCompacting = false,
  sessionId,
  hasCustomPrompt = false,
  onCustomPromptChange,
  rewindDraft,
  onRewindDraftConsumed,
  mcpMode = 'disabled',
  onMcpModeChange,
  mcpServerList = [],
  selectedMcpServers = [],
  onMcpServerToggle,
}: InputAreaProps) {
  const [commandMode, setCommandMode] = useState(false);
  const [promptDialogOpen, setPromptDialogOpen] = useState(false);
  const [providerDropdownOpen, setProviderDropdownOpen] = useState(false);
  const [kbPickerOpen, setKbPickerOpen] = useState(false);
  const [selectedKbCollections, setSelectedKbCollections] = useState<string[]>([]);
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [thinkingDropdownOpen, setThinkingDropdownOpen] = useState(false);
  const [mcpDropdownOpen, setMcpDropdownOpen] = useState(false);
  const contentEditableRef = useRef<ContentEditableInputHandle>(null);
  const providerDropdownRef = useRef<HTMLDivElement>(null);
  const kbPickerRef = useRef<HTMLDivElement>(null);
  const thinkingDropdownRef = useRef<HTMLDivElement>(null);
  const mcpDropdownRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(false);
  const lastCompEndRef = useRef<number>(0);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [translating, setTranslating] = useState(false);
  const [inputHasText, setInputHasText] = useState(false);

  // Plan mode: defaults to a global preference, but can be controlled by a caller.
  const [uncontrolledPlanMode, setUncontrolledPlanMode] = useState<PlanMode>(() => {
    const stored = localStorage.getItem('y-agent-plan-mode');
    if (stored === 'fast' || stored === 'auto' || stored === 'plan') return stored;
    return 'fast';
  });
  const planMode = controlledPlanMode ?? uncontrolledPlanMode;
  const cyclePlanMode = useCallback(() => {
    const next: PlanMode = planMode === 'fast' ? 'auto' : planMode === 'auto' ? 'plan' : 'fast';
    if (controlledPlanMode !== undefined) {
      onPlanModeChange?.(next);
      return;
    }
    setUncontrolledPlanMode(next);
    if (persistPlanMode) {
      localStorage.setItem('y-agent-plan-mode', next);
    }
  }, [controlledPlanMode, onPlanModeChange, persistPlanMode, planMode]);

  // Close provider dropdown on outside click.
  useEffect(() => {
    if (!providerDropdownOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (providerDropdownRef.current && !providerDropdownRef.current.contains(e.target as Node)) {
        setProviderDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [providerDropdownOpen]);

  // Close knowledge picker on outside click.
  useEffect(() => {
    if (!kbPickerOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (kbPickerRef.current && !kbPickerRef.current.contains(e.target as Node)) {
        setKbPickerOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [kbPickerOpen]);

  // Close thinking dropdown on outside click.
  useEffect(() => {
    if (!thinkingDropdownOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (thinkingDropdownRef.current && !thinkingDropdownRef.current.contains(e.target as Node)) {
        setThinkingDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [thinkingDropdownOpen]);

  // Close MCP dropdown on outside click.
  useEffect(() => {
    if (!mcpDropdownOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (mcpDropdownRef.current && !mcpDropdownRef.current.contains(e.target as Node)) {
        setMcpDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [mcpDropdownOpen]);

  // Derive display label for selected provider.
  const selectedProviderLabel = selectedProviderId === 'auto'
    ? 'Auto'
    : providers.find((p) => p.id === selectedProviderId)?.model || selectedProviderId;

  const updateHasContent = useCallback(() => {
    const hasContent = contentEditableRef.current?.hasContent() ?? false;
    setInputHasText(hasContent);
  }, []);

  const resetInput = useCallback(() => {
    contentEditableRef.current?.clear();
  }, []);

  const exitCommandMode = useCallback(() => {
    setCommandMode(false);
  }, []);

  const handleCommandSelect = useCallback(
    (cmd: GuiCommandDef) => {
      exitCommandMode();
      if (cmd.immediate) {
        resetInput();
        onCommand?.(cmd.name);
      } else if (cmd.name === 'model') {
        // model command enters arg mode inside CommandMenu; don't exit yet.
      } else {
        contentEditableRef.current?.setText(`/${cmd.name} `);
        contentEditableRef.current?.placeCursorAtEnd();
        updateHasContent();
      }
    },
    [onCommand, resetInput, exitCommandMode, updateHasContent],
  );

  const handleSkillSelect = useCallback(
    (skillName: string) => {
      exitCommandMode();
      contentEditableRef.current?.removeSlashCommandText();
      contentEditableRef.current?.insertSkillMention(skillName);
      updateHasContent();
    },
    [exitCommandMode, updateHasContent],
  );

  const toggleKbCollection = useCallback((name: string) => {
    setSelectedKbCollections(prev =>
      prev.includes(name) ? prev.filter(n => n !== name) : [...prev, name]
    );
  }, []);

  const handleModelProviderSelect = useCallback(
    (providerId: string) => {
      onSelectProvider(providerId);
      exitCommandMode();
      resetInput();
    },
    [onSelectProvider, exitCommandMode, resetInput],
  );

  const handleKbCollectionSelect = useCallback(
    (collectionName: string) => {
      exitCommandMode();
      contentEditableRef.current?.removeSlashCommandText();
      toggleKbCollection(collectionName);
      contentEditableRef.current?.placeCursorAtEnd();
      contentEditableRef.current?.focus();
    },
    [exitCommandMode, toggleKbCollection],
  );

  const clearKbSelections = useCallback(() => {
    setSelectedKbCollections([]);
  }, []);

  const handleSend = useCallback(() => {
    if (disabled) return;
    // Prevent double-send from rapid Enter key events (common on Windows).
    if (sendingRef.current) return;

    const { text, skills: extractedSkills } = contentEditableRef.current?.extractContent() ?? { text: '', skills: [] };
    const trimmed = text.trim();

    if (!trimmed && extractedSkills.length === 0 && selectedKbCollections.length === 0 && attachments.length === 0) return;

    // Intercept slash commands.
    if (trimmed.startsWith('/')) {
      const parts = trimmed.slice(1).split(/\s+/);
      const cmdName = parts[0];
      if (cmdName && onCommand?.(cmdName)) {
        resetInput();
        exitCommandMode();
        return;
      }
    }

    sendingRef.current = true;

    onSend(
      trimmed,
      extractedSkills.length > 0 ? extractedSkills : undefined,
      selectedKbCollections.length > 0 ? selectedKbCollections : undefined,
      thinkingEffort,
      attachments.length > 0 ? attachments : undefined,
      planMode,
      mcpMode,
      mcpMode === 'manual' ? selectedMcpServers : undefined,
    );
    resetInput();
    setAttachments([]);
    exitCommandMode();
    // Release on next microtask so any queued keydown events are still blocked.
    queueMicrotask(() => { sendingRef.current = false; });
  }, [disabled, onSend, onCommand, resetInput, exitCommandMode, selectedKbCollections, thinkingEffort, attachments, planMode, mcpMode, selectedMcpServers]);

  const handlePaste = useCallback(async (e: React.ClipboardEvent) => {
    // Check for pasted images first.
    const items = e.clipboardData.items;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) return;
        const buffer = await file.arrayBuffer();
        const base64 = btoa(
          new Uint8Array(buffer).reduce((data, byte) => data + String.fromCharCode(byte), ''),
        );
        const ext = file.type.split('/')[1] || 'png';
        const att: Attachment = {
          id: `paste-${Date.now()}`,
          filename: `pasted-image.${ext}`,
          mime_type: file.type,
          base64_data: base64,
          size: file.size,
        };
        setAttachments((prev) => [...prev, att]);
        return;
      }
    }
    // Fallback: paste as plain text.
    e.preventDefault();
    const text = e.clipboardData.getData('text/plain');
    // NOTE: execCommand is deprecated but is the only API that inserts text
    // into a contenteditable while preserving the native undo stack.
    document.execCommand('insertText', false, text);
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Ignore key events during IME composition (e.g. Chinese pinyin input).
    if (e.nativeEvent.isComposing) {
      return;
    }

    // On some platforms a single Enter press during composition can fire two 
    // keydown events (isComposing=true then isComposing=false).
    // To prevent it, we ignore keydown events immediately after composition ends.
    if (Date.now() - lastCompEndRef.current < 100) {
      return;
    }

    // In command mode, let the CommandMenu handle most keyboard events.
    if (commandMode) {
      if (e.key === 'Escape') {
        e.preventDefault();
        exitCommandMode();
        resetInput();
        return;
      }
      // When Enter is pressed in the editable while command mode is active,
      // extract the slash command from the editable text and dispatch it
      // directly. This handles the case where the user types `/compact`
      // fast and presses Enter before the CommandMenu search captures focus.
      if (sendOnEnter && e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        const plainText = contentEditableRef.current?.getPlainText().trim() ?? '';
        if (plainText.startsWith('/')) {
          const cmdName = plainText.slice(1).split(/\s+/)[0];
          if (cmdName && onCommand?.(cmdName)) {
            resetInput();
            exitCommandMode();
            return;
          }
        }
        // Fallback: treat as normal send (exits command mode via handleSend).
        handleSend();
      }
      return;
    }

    // Normal mode: Enter to send.
    if (sendOnEnter && e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = useCallback((plainText: string) => {
    updateHasContent();

    if (plainText.startsWith('/') && !plainText.includes('\n')) {
      const query = plainText.slice(1).split(/\s+/)[0];
      const hasMatch =
        GUI_COMMANDS.some(
          (cmd) => cmd.name.startsWith(query) || (cmd.alias !== null && cmd.alias.startsWith(query)),
        ) ||
        skills.some(
          (s) =>
            s.name.toLowerCase().startsWith(query) ||
            s.tags.some((t) => t.toLowerCase().startsWith(query)),
        ) ||
        knowledgeCollections.some(
          (c) => c.name.toLowerCase().startsWith(query),
        );
      if (hasMatch) {
        setCommandMode(true);
      } else if (commandMode) {
        exitCommandMode();
      }
    } else {
      if (commandMode) exitCommandMode();
    }
  }, [commandMode, exitCommandMode, updateHasContent, skills, knowledgeCollections]);

  // When entering edit mode, populate with the message content.
  useEffect(() => {
    if (pendingEdit) {
      contentEditableRef.current?.setText(pendingEdit.content);
      // Defer state update to avoid cascading render inside effect.
      queueMicrotask(exitCommandMode);
      contentEditableRef.current?.focus();
      contentEditableRef.current?.placeCursorAtEnd();
      updateHasContent();
    }
  }, [pendingEdit, exitCommandMode, updateHasContent]);

  // Populate input with draft text from rewind/undo operations.
  useEffect(() => {
    if (rewindDraft) {
      contentEditableRef.current?.setText(rewindDraft);
      // Defer state update to avoid cascading render inside effect.
      queueMicrotask(exitCommandMode);
      contentEditableRef.current?.focus();
      contentEditableRef.current?.placeCursorAtEnd();
      updateHasContent();
      onRewindDraftConsumed?.();
    }
  }, [rewindDraft, exitCommandMode, updateHasContent, onRewindDraftConsumed]);

  const handleTranslate = useCallback(async () => {
    if (translating) return;
    const { text } = contentEditableRef.current?.extractContent() ?? { text: '' };
    if (!text.trim()) return;
    setTranslating(true);
    try {
      const translated = await transport.invoke<string>('translate_text', { text: text.trim() });
      contentEditableRef.current?.setText(translated);
      contentEditableRef.current?.placeCursorAtEnd();
      updateHasContent();
    } catch (e) {
      console.error('[InputArea] translation error:', e);
    } finally {
      setTranslating(false);
    }
  }, [translating, updateHasContent]);

  return (
    <div className={`input-area ${expanded ? 'input-area--expanded' : ''}`}>
      {pendingEdit && (
        <div className="edit-banner">
          <span className="edit-banner-text">Editing message -- sending will undo context to this point</span>
          <button
            className="edit-banner-dismiss"
            onClick={onCancelEdit}
            title="Cancel edit"
            aria-label="Cancel edit"
          >
            <X size={14} />
          </button>
        </div>
      )}
      <div className="input-container">
        {commandMode && (
          <CommandMenu
            skills={skills}
            knowledgeCollections={knowledgeCollections}
            providers={providers}
            selectedProviderId={selectedProviderId}
            onSelect={handleCommandSelect}
            onSelectSkill={handleSkillSelect}
            onSelectKbCollection={handleKbCollectionSelect}
            onSelectModelProvider={handleModelProviderSelect}
            onDismiss={() => {
              exitCommandMode();
              resetInput();
            }}
          />
        )}

        {askUserData && onAskUserSubmit && onAskUserDismiss && (
          <AskUserDialog
            interactionId={askUserData.interactionId}
            questions={askUserData.questions}
            onSubmit={onAskUserSubmit}
            onDismiss={onAskUserDismiss}
          />
        )}

        {permissionData && onPermissionApprove && onPermissionDeny && onPermissionAllowAllForSession && (
          <PermissionDialog
            requestId={permissionData.requestId}
            toolName={permissionData.toolName}
            actionDescription={permissionData.actionDescription}
            reason={permissionData.reason}
            contentPreview={permissionData.contentPreview}
            onApprove={onPermissionApprove}
            onDeny={onPermissionDeny}
            onAllowAllForSession={onPermissionAllowAllForSession}
          />
        )}

        {/* Editable div with inline skill mentions */}
        <ContentEditableInput
          ref={contentEditableRef}
          disabled={disabled}
          translating={translating}
          isCompacting={isCompacting}
          attachments={attachments}
          onRemoveAttachment={(id) => setAttachments((prev) => prev.filter((a) => a.id !== id))}
          onInput={handleInput}
          onPaste={handlePaste}
          onKeyDown={handleKeyDown}
          onCompositionEnd={() => {
            lastCompEndRef.current = Date.now();
          }}
          lastCompEndRef={lastCompEndRef}
        />

        {isCompacting && (
          <div className="btn-compacting" title="Compacting context...">
            <Loader2 size={16} className="compacting-spinner" />
          </div>
        )}

        {!isCompacting && disabled && onStop && (
          <button
            className="btn-stop"
            onClick={onStop}
            title="Stop generation"
            id="btn-stop-generation"
          >
            <Square size={14} />
          </button>
        )}

        {/* Toolbar row with action buttons -- inside the input border */}
        <div
          className={`input-toolbar${
            providerDropdownOpen || kbPickerOpen || thinkingDropdownOpen || mcpDropdownOpen
              ? ' input-toolbar--dropdown-open'
              : ''
          }`}
        >

          {/* (b) Model / provider selection */}
          <div className="toolbar-btn-group" ref={providerDropdownRef}>
            <button
              className="toolbar-btn has-tooltip"
              onClick={() => setProviderDropdownOpen(!providerDropdownOpen)}
              data-tooltip="Select model"
              disabled={disabled}
            >
              <AtSign size={14} />
              <span className="toolbar-btn-label">{selectedProviderLabel}</span>
            </button>
            {providerDropdownOpen && (
              <div className="toolbar-provider-dropdown">
                <button
                  className={`toolbar-provider-item ${selectedProviderId === 'auto' ? 'selected' : ''}`}
                  onClick={() => { onSelectProvider('auto'); setProviderDropdownOpen(false); }}
                >
                  Auto
                </button>
                {providers.map((p) => (
                  <button
                    key={p.id}
                    className={`toolbar-provider-item ${selectedProviderId === p.id ? 'selected' : ''}`}
                    onClick={() => { onSelectProvider(p.id); setProviderDropdownOpen(false); }}
                  >
                    {providerIcons?.[p.id] ? (
                      <ProviderIconImg iconId={providerIcons[p.id]} size={14} className="toolbar-provider-icon" />
                    ) : (
                      <Bot size={14} className="toolbar-provider-icon toolbar-provider-icon--default" />
                    )}
                    {p.id} ({p.model})
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* (f) Plan mode selector */}
          <button
            className={`toolbar-btn has-tooltip toolbar-btn--plan-${planMode}`}
            onClick={cyclePlanMode}
            data-tooltip={`Mode: ${planMode}`}
            disabled={disabled}
          >
            {planMode === 'fast' && <Zap size={14} />}
            {planMode === 'auto' && <ScanSearch size={14} />}
            {planMode === 'plan' && <ClipboardList size={14} />}
            <span className="toolbar-btn-label">{planMode}</span>
          </button>
          {/* (a) Attachment picker */}
          <button
            className={`toolbar-btn has-tooltip ${attachments.length > 0 ? 'toolbar-btn--active' : ''}`}
            onClick={async () => {
              try {
                const result = await platform.openFileDialog({
                  multiple: true,
                  filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }],
                });
                if (result) {
                  const paths = result;
                  const atts = await transport.invoke<Attachment[]>('attachment_read_files', { paths });
                  setAttachments((prev) => [...prev, ...atts]);
                }
              } catch (e) {
                console.error('[InputArea] attachment picker error:', e);
              }
            }}
            data-tooltip="Attach images"
            disabled={disabled}
          >
            <Paperclip size={14} />
            {attachments.length > 0 && (
              <span className="toolbar-btn-label">{attachments.length}</span>
            )}
          </button>

          {/* (c) Clear all messages */}
          <button
            className="toolbar-btn toolbar-btn--danger has-tooltip"
            onClick={() => setClearConfirmOpen(true)}
            data-tooltip="Clear chat"
            disabled={disabled}
          >
            <Paintbrush size={14} />
          </button>

          {/* (d) Add context reset */}
          <button
            className="toolbar-btn has-tooltip"
            onClick={onAddContextReset}
            data-tooltip="Reset context"
            disabled={disabled}
          >
            <Eraser size={14} />
          </button>

          {/* (d2) Session custom prompt */}
          <button
            className={`toolbar-btn has-tooltip ${hasCustomPrompt ? 'toolbar-btn--active' : ''}`}
            onClick={() => setPromptDialogOpen(true)}
            data-tooltip="Session prompt"
            disabled={disabled || !sessionId}
          >
            <ScrollText size={14} />
          </button>

          {/* (e) Thinking effort selector */}
          <div className="toolbar-btn-group" ref={thinkingDropdownRef}>
            <button
              className={`toolbar-btn has-tooltip ${thinkingEffort ? 'toolbar-btn--active' : ''}`}
              onClick={() => setThinkingDropdownOpen(!thinkingDropdownOpen)}
              data-tooltip="Thinking effort"
              disabled={disabled}
            >
              <Lightbulb size={14} />
              {thinkingEffort && (
                <span className="toolbar-btn-label">{thinkingEffort}</span>
              )}
            </button>
            {thinkingDropdownOpen && (
              <div className="toolbar-thinking-dropdown">
                {([null, 'low', 'medium', 'high', 'max'] as const).map((level) => (
                  <button
                    key={level ?? 'default'}
                    className={`toolbar-thinking-item ${(thinkingEffort ?? null) === level ? 'selected' : ''}`}
                    onClick={() => {
                      onThinkingEffortChange?.(level);
                      setThinkingDropdownOpen(false);
                    }}
                  >
                    {level === null ? 'Default' : level.charAt(0).toUpperCase() + level.slice(1)}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* (g) MCP mode selector */}
          <div className="toolbar-btn-group" ref={mcpDropdownRef}>
            <button
              className={`toolbar-btn has-tooltip ${mcpMode !== 'disabled' ? 'toolbar-btn--active' : ''}`}
              onClick={() => setMcpDropdownOpen(!mcpDropdownOpen)}
              data-tooltip="MCP mode"
              disabled={disabled}
            >
              <Cpu size={14} />
            </button>
            {mcpDropdownOpen && (
              <div className="toolbar-mcp-dropdown">
                <div className="toolbar-mcp-section">
                  {(['auto', 'manual', 'disabled'] as const).map((m) => (
                    <button
                      key={m}
                      className={`toolbar-mcp-mode-item ${mcpMode === m ? 'selected' : ''}`}
                      onClick={() => {
                        onMcpModeChange?.(m);
                        if (m !== 'manual') setMcpDropdownOpen(false);
                      }}
                    >
                      <span className="toolbar-mcp-mode-label">{m.charAt(0).toUpperCase() + m.slice(1)}</span>
                      <span className="toolbar-mcp-mode-desc">
                        {m === 'auto' && 'All enabled MCP servers'}
                        {m === 'manual' && 'Choose servers'}
                        {m === 'disabled' && 'No MCP tools'}
                      </span>
                    </button>
                  ))}
                </div>
                {mcpMode === 'manual' && (
                  <div className="toolbar-mcp-section toolbar-mcp-section--servers">
                    <div className="toolbar-mcp-section-title">Servers</div>
                    {mcpServerList.length === 0 ? (
                      <div className="toolbar-mcp-empty">No MCP servers configured</div>
                    ) : (
                      mcpServerList.map((srv) => (
                        <button
                          key={srv.name}
                          className={`toolbar-mcp-server-item ${selectedMcpServers.includes(srv.name) ? 'selected' : ''}`}
                          onClick={() => onMcpServerToggle?.(srv.name)}
                          disabled={srv.disabled}
                        >
                          <span className="toolbar-mcp-server-check">
                            {selectedMcpServers.includes(srv.name) ? '\u2713' : ''}
                          </span>
                          <span className="toolbar-mcp-server-name">{srv.name}</span>
                          {srv.disabled && <span className="toolbar-mcp-server-off">OFF</span>}
                        </button>
                      ))
                    )}
                  </div>
                )}
              </div>
            )}
          </div>

          {/* (f) Knowledge base picker */}
          {knowledgeCollections.length > 0 && (
            <div className="toolbar-btn-group" ref={kbPickerRef}>
              <button
                className={`toolbar-btn has-tooltip ${selectedKbCollections.length > 0 ? 'toolbar-btn--active' : ''}`}
                onClick={() => setKbPickerOpen(!kbPickerOpen)}
                data-tooltip="Knowledge bases"
                disabled={disabled}
              >
                <BookOpen size={14} />
                {selectedKbCollections.length > 0 && (
                  <span className="toolbar-btn-label">{selectedKbCollections.length} selected</span>
                )}
              </button>
              {kbPickerOpen && (
                <div className="toolbar-kb-dropdown">
                  <div className="toolbar-kb-header">
                    <span className="toolbar-kb-title">Knowledge Bases</span>
                    {selectedKbCollections.length > 0 && (
                      <button
                        className="toolbar-kb-clear"
                        onClick={clearKbSelections}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                  {knowledgeCollections.map((col) => (
                    <div
                      key={col.id}
                      className={`toolbar-kb-item ${selectedKbCollections.includes(col.name) ? 'selected' : ''}`}
                      onClick={() => toggleKbCollection(col.name)}
                    >
                      <span className="toolbar-kb-item-name">{col.name}</span>
                      <span className="toolbar-kb-item-count">{col.entry_count} entries</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            
          )}

          {/* Translate input text */}
          <button
            className="toolbar-btn has-tooltip"
            onClick={handleTranslate}
            data-tooltip="Translate"
            disabled={disabled || !inputHasText || translating}
          >
            <Languages size={14} />
          </button>

          {/* (b) Expand / collapse input */}
          <button
            className="toolbar-btn has-tooltip"
            onClick={() => onExpandChange?.(!expanded)}
            data-tooltip={expanded ? 'Collapse input' : 'Expand input'}
          >
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </button>
        </div>
      </div>

      <ConfirmDialog
        open={clearConfirmOpen}
        title="Clear all messages"
        message="This will permanently delete the current conversation. This cannot be undone."
        confirmLabel="Clear"
        variant="danger"
        onConfirm={() => {
          setClearConfirmOpen(false);
          onClearSession?.();
        }}
        onCancel={() => setClearConfirmOpen(false)}
      />

      {promptDialogOpen && sessionId && (
        <SessionPromptDialog
          sessionId={sessionId}
          onClose={() => setPromptDialogOpen(false)}
          onSaved={(hasPrompt) => {
            setPromptDialogOpen(false);
            onCustomPromptChange?.(hasPrompt);
          }}
        />
      )}
    </div>
  );
}
