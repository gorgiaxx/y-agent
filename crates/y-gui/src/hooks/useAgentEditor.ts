import { useState, useCallback, useEffect } from 'react';
import type { AgentDetail } from './useAgents';
import type { EditorTab, EditorSurface, AgentDraft } from '../components/agents/types';
import { buildDraft, serializeAgentDraft, slugifyAgentId } from '../components/agents/utils';

interface UseAgentEditorParams {
  getAgentDetail: (id: string) => Promise<AgentDetail | null>;
  getAgentSource: (id: string) => Promise<{ content: string; path: string | null; is_user_file: boolean } | null>;
  parseAgentToml: (toml: string) => Promise<AgentDetail | null>;
  saveAgent: (id: string, content: string) => Promise<boolean>;
  resetAgent: (id: string) => Promise<boolean>;
}

export function useAgentEditor({
  getAgentDetail,
  getAgentSource,
  parseAgentToml,
  saveAgent,
  resetAgent,
}: UseAgentEditorParams) {
  const [editorOpen, setEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<'create' | 'edit'>('create');
  const [editorTab, setEditorTab] = useState<EditorTab>('general');
  const [editorSurface, setEditorSurface] = useState<EditorSurface>('form');
  const [editorDraft, setEditorDraft] = useState<AgentDraft>(buildDraft());
  const [editorRawToml, setEditorRawToml] = useState('');
  const [editorRawPath, setEditorRawPath] = useState<string | null>(null);
  const [editorRawUsesSourceFile, setEditorRawUsesSourceFile] = useState(false);
  const [editorRawOrigin, setEditorRawOrigin] = useState<'form' | 'raw' | 'source'>('form');
  const [editorRawError, setEditorRawError] = useState<string | null>(null);
  const [editorSaving, setEditorSaving] = useState(false);

  const handleOpenCreate = useCallback(() => {
    const draft = buildDraft();
    setEditorMode('create');
    setEditorTab('general');
    setEditorSurface('form');
    setEditorDraft(draft);
    setEditorRawToml(serializeAgentDraft(draft));
    setEditorRawPath(null);
    setEditorRawUsesSourceFile(false);
    setEditorRawOrigin('form');
    setEditorRawError(null);
    setEditorOpen(true);
  }, []);

  const handleOpenEdit = useCallback(async (agentId: string) => {
    const [detail, source] = await Promise.all([
      getAgentDetail(agentId),
      getAgentSource(agentId),
    ]);
    if (!detail) return;
    setEditorMode('edit');
    setEditorTab('general');
    setEditorSurface('form');
    setEditorDraft(buildDraft(detail));
    setEditorRawToml(source?.content ?? serializeAgentDraft(buildDraft(detail)));
    setEditorRawPath(source?.path ?? null);
    setEditorRawUsesSourceFile(source?.is_user_file ?? false);
    setEditorRawOrigin(source ? 'source' : 'form');
    setEditorRawError(null);
    setEditorOpen(true);
  }, [getAgentDetail, getAgentSource]);

  const handleApplyTemplate = useCallback(async (agentId: string) => {
    const detail = await getAgentDetail(agentId);
    if (!detail) return;
    const nextDraft = {
      ...buildDraft(detail),
      id: '',
      name: `Copy ${detail.name}`,
    };
    setEditorDraft(nextDraft);
    setEditorSurface('form');
    setEditorRawToml(serializeAgentDraft(nextDraft));
    setEditorRawPath(null);
    setEditorRawUsesSourceFile(false);
    setEditorRawOrigin('form');
    setEditorRawError(null);
  }, [getAgentDetail]);

  const handleEditorDraftChange = useCallback((updater: (draft: AgentDraft) => AgentDraft) => {
    setEditorRawOrigin('form');
    setEditorRawError(null);
    setEditorDraft((prev) => updater(prev));
  }, []);

  useEffect(() => {
    if (!editorOpen || editorSurface !== 'form' || editorRawOrigin !== 'form') {
      return;
    }
    setEditorRawToml(serializeAgentDraft(editorDraft));
  }, [editorDraft, editorOpen, editorRawOrigin, editorSurface]);

  const handleEditorSurfaceChange = useCallback(async (surface: EditorSurface) => {
    if (surface === editorSurface) {
      return;
    }

    setEditorRawError(null);

    if (surface === 'raw') {
      setEditorSurface('raw');
      return;
    }

    const parsed = await parseAgentToml(editorRawToml);
    if (!parsed) {
      setEditorRawError('Raw TOML has syntax or schema errors. Fix it before returning to the form editor.');
      return;
    }

    setEditorDraft(buildDraft(parsed));
    setEditorRawOrigin('form');
    setEditorSurface('form');
  }, [editorRawToml, editorSurface, parseAgentToml]);

  const handleSaveEditor = useCallback(async (): Promise<boolean> => {
    let nextId = editorMode === 'edit' ? editorDraft.id : (editorDraft.id.trim() || slugifyAgentId(editorDraft.name));
    let nextContent = serializeAgentDraft({
      ...editorDraft,
      id: nextId,
    });

    if (editorSurface === 'raw') {
      const parsed = await parseAgentToml(editorRawToml);
      if (!parsed) {
        setEditorRawError('Raw TOML has syntax or schema errors. Fix it before saving.');
        return false;
      }

      if (editorMode === 'edit') {
        if (parsed.id.trim() && parsed.id.trim() !== editorDraft.id) {
          setEditorRawError('Existing agent IDs cannot be changed in raw mode.');
          return false;
        }
        nextId = editorDraft.id;
      } else {
        nextId = parsed.id.trim();
      }

      if (!nextId || !parsed.name.trim()) {
        setEditorRawError('Raw TOML must include both non-empty id and name fields before saving.');
        return false;
      }

      nextContent = editorRawToml;
    } else if (!nextId || !editorDraft.name.trim()) {
      return false;
    }

    setEditorSaving(true);
    const ok = await saveAgent(nextId, nextContent);
    setEditorSaving(false);
    if (!ok) return false;

    setEditorOpen(false);
    return true;
  }, [editorDraft, editorMode, editorRawToml, editorSurface, parseAgentToml, saveAgent]);

  const handleResetEditor = useCallback(async (): Promise<boolean> => {
    if (editorMode !== 'edit') return false;

    const ok = await resetAgent(editorDraft.id);
    if (!ok) return false;

    setEditorOpen(false);
    return true;
  }, [editorDraft.id, editorMode, resetAgent]);

  return {
    editorOpen,
    editorMode,
    editorTab,
    editorSurface,
    editorDraft,
    editorRawToml,
    editorRawPath,
    editorRawUsesSourceFile,
    editorRawOrigin,
    editorRawError,
    editorSaving,
    setEditorTab,
    setEditorRawToml: (content: string) => {
      setEditorRawToml(content);
      setEditorRawOrigin('raw');
      setEditorRawError(null);
    },
    handleOpenCreate,
    handleOpenEdit,
    handleApplyTemplate,
    handleEditorDraftChange,
    handleEditorSurfaceChange,
    handleSaveEditor,
    handleResetEditor,
    closeEditor: () => setEditorOpen(false),
  };
}
