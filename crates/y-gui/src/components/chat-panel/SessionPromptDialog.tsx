import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Button,
} from '../ui';

interface SessionPromptDialogProps {
  sessionId: string;
  onClose: () => void;
  onSaved: (hasPrompt: boolean) => void;
}

export function SessionPromptDialog({
  sessionId,
  onClose,
  onSaved,
}: SessionPromptDialogProps) {
  const [prompt, setPrompt] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const loadPrompt = useCallback(async () => {
    setLoading(true);
    try {
      const current = await invoke<string | null>('session_get_custom_prompt', {
        sessionId,
      });
      setPrompt(current ?? '');
    } catch {
      setPrompt('');
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    loadPrompt();
  }, [loadPrompt]);

  const handleSave = async () => {
    setSaving(true);
    try {
      const value = prompt.trim() || null;
      await invoke('session_set_custom_prompt', {
        sessionId,
        prompt: value,
      });
      onSaved(value !== null);
    } catch (e) {
      console.error('Failed to save custom prompt:', e);
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async () => {
    setSaving(true);
    try {
      await invoke('session_set_custom_prompt', {
        sessionId,
        prompt: null,
      });
      setPrompt('');
      onSaved(false);
    } catch (e) {
      console.error('Failed to clear custom prompt:', e);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent width="560px" className="text-left items-stretch">
        <DialogTitle className="text-left">
          Session System Prompt
        </DialogTitle>

        <div className="flex flex-col gap-2 mt-2">
          <p className="text-11px text-[var(--text-muted)] m-0">
            Override the built-in system prompt for this session.
            Dynamic sections (tool protocol, plan mode, datetime, environment) are preserved.
            Leave empty to use the global default.
          </p>

          {loading ? (
            <div className="text-12px text-[var(--text-muted)] py-4 text-center">
              Loading...
            </div>
          ) : (
            <textarea
              className="toml-editor prompt-editor"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              spellCheck={false}
              placeholder="Enter custom system prompt for this session..."
              style={{ minHeight: '200px', maxHeight: '400px', resize: 'vertical' }}
            />
          )}

          <div className="flex gap-2 justify-between mt-1">
            <Button
              type="button"
              variant="ghost"
              onClick={handleClear}
              disabled={loading || saving || !prompt.trim()}
            >
              Clear
            </Button>
            <div className="flex gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="primary"
                onClick={handleSave}
                disabled={loading || saving}
              >
                {saving ? 'Saving...' : 'Save'}
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
