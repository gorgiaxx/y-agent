// ---------------------------------------------------------------------------
// PromptsTab -- Builtin prompts editor with sidebar list
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { RotateCcw } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { MonacoEditor } from '../ui/MonacoEditor';

interface PromptsTabProps {
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
  dirtyPrompts: Record<string, string>;
  setDirtyPrompts: React.Dispatch<React.SetStateAction<Record<string, string>>>;
}

/** Friendly label: strip "core_" prefix and ".txt" suffix. */
function promptLabel(filename: string): string {
  return filename.replace(/^core_/, '').replace(/\.txt$/, '');
}

export function PromptsTab({
  setToast,
  dirtyPrompts,
  setDirtyPrompts,
}: PromptsTabProps) {
  const [promptFiles, setPromptFiles] = useState<string[]>([]);
  const [activePromptTab, setActivePromptTab] = useState(0);
  const [promptContent, setPromptContent] = useState('');
  const [promptLoading, setPromptLoading] = useState(false);

  const loadPromptFile = useCallback(async (filename: string) => {
    setPromptLoading(true);
    try {
      const content = await invoke<string>('prompt_get', { filename });
      setPromptContent(content);
    } catch (e) {
      setToast({ message: `Failed to load prompt: ${e}`, type: 'error' });
    } finally {
      setPromptLoading(false);
    }
  }, [setToast]);

  const loadPromptFiles = useCallback(async () => {
    setPromptLoading(true);
    try {
      const files = await invoke<string[]>('prompt_list');
      setPromptFiles(files);
      setActivePromptTab(0);
      if (files.length > 0) {
        // Check if there's a dirty draft for this file.
        const firstFile = files[0];
        if (dirtyPrompts[firstFile] !== undefined) {
          setPromptContent(dirtyPrompts[firstFile]);
          setPromptLoading(false);
        } else {
          await loadPromptFile(firstFile);
        }
      } else {
        setPromptContent('');
        setPromptLoading(false);
      }
    } catch (e) {
      setToast({ message: `Failed to list prompts: ${e}`, type: 'error' });
      setPromptLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadPromptFile]);

  useEffect(() => {
    loadPromptFiles();
  }, [loadPromptFiles]);

  const handlePromptTabSwitch = useCallback(async (index: number) => {
    setActivePromptTab(index);
    const filename = promptFiles[index];
    if (!filename) return;
    // Use dirty draft if available, otherwise load from disk.
    if (dirtyPrompts[filename] !== undefined) {
      setPromptContent(dirtyPrompts[filename]);
    } else {
      await loadPromptFile(filename);
    }
  }, [promptFiles, dirtyPrompts, loadPromptFile]);

  const handlePromptRestore = useCallback(async () => {
    const filename = promptFiles[activePromptTab];
    if (!filename) return;
    try {
      const defaultContent = await invoke<string>('prompt_get_default', { filename });
      setPromptContent(defaultContent);
      setDirtyPrompts((prev) => ({ ...prev, [filename]: defaultContent }));
      setToast({ message: `Restored "${promptLabel(filename)}" to default`, type: 'success' });
    } catch (e) {
      setToast({ message: `Restore failed: ${e}`, type: 'error' });
    }
  }, [promptFiles, activePromptTab, setDirtyPrompts, setToast]);

  return (
    <div className="settings-section settings-section--fill">
      <div className="settings-header">
        <h3 className="section-title section-title--flush">
          Builtin Prompts
        </h3>
      </div>
      {promptLoading && promptFiles.length === 0 ? (
        <div className="section-loading">Loading...</div>
      ) : promptFiles.length === 0 ? (
        <div className="settings-empty">
          No prompt files found. Run&nbsp;<code>y-agent init</code>&nbsp;to seed defaults.
        </div>
      ) : (
        <div className="sub-list-layout">
          {/* Left sidebar list -- one item per prompt file */}
          <div className="sub-list-sidebar">
            <div className="sub-list-items">
              {promptFiles.map((f, i) => (
                <button
                  key={f}
                  className={`sub-list-item ${activePromptTab === i ? 'active' : ''}`}
                  onClick={() => handlePromptTabSwitch(i)}
                >
                  <span className="sub-list-item-label">{promptLabel(f)}</span>
                </button>
              ))}
            </div>
          </div>

          {/* Right detail panel -- prompt editor */}
          <div className="sub-list-detail sub-list-detail--editor">
            {promptLoading ? (
              <div className="section-loading">Loading...</div>
            ) : (
              <div className="toml-editor-wrap">
                <MonacoEditor
                  className="prompt-editor-monaco"
                  value={promptContent}
                  onChange={(val) => {
                    setPromptContent(val);
                    const filename = promptFiles[activePromptTab];
                    if (filename) {
                      setDirtyPrompts((prev) => ({ ...prev, [filename]: val }));
                    }
                  }}
                  language="plaintext"
                  placeholder="Empty prompt. Type content here."
                />
                <div className="prompt-editor-actions">
                  <button
                    type="button"
                    className="btn-prompt-restore"
                    onClick={handlePromptRestore}
                    title="Restore to default"
                  >
                    <RotateCcw size={13} />
                    <span>Restore Default</span>
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
