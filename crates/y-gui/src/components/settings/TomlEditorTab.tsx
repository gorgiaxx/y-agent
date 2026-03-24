// ---------------------------------------------------------------------------
// TomlEditorTab -- Raw TOML editor for sections without structured forms
// (storage, hooks, tools, guardrails, knowledge)
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import type { SettingsTab } from './SettingsPanel';
import { maskSensitive } from './settingsTypes';

interface TomlEditorTabProps {
  activeTab: SettingsTab;
  loadSection: (section: string) => Promise<string>;
  setToast: (toast: { message: string; type: 'success' | 'error' } | null) => void;
  tomlDraftsBySection: Record<string, string>;
  setTomlDraftsBySection: React.Dispatch<React.SetStateAction<Record<string, string>>>;
}

export function TomlEditorTab({
  activeTab,
  loadSection,
  setToast,
  tomlDraftsBySection,
  setTomlDraftsBySection,
}: TomlEditorTabProps) {
  const [sectionContent, setSectionContent] = useState('');
  const [rawContent, setRawContent] = useState('');
  const [sectionLoading, setSectionLoading] = useState(false);

  const doLoadSection = useCallback(
    async (section: string) => {
      setSectionLoading(true);
      try {
        const content = await loadSection(section);
        setRawContent(content);
        setSectionContent(content);
      } catch (e) {
        setToast({ message: `Failed to load: ${e}`, type: 'error' });
      } finally {
        setSectionLoading(false);
      }
    },
    [loadSection, setToast],
  );

  useEffect(() => {
    doLoadSection(activeTab);
  }, [activeTab, doLoadSection]);

  return (
    <>
      {sectionLoading ? (
        <div className="section-loading">Loading...</div>
      ) : (
        
        <div className="toml-editor-wrap">
          <textarea
            className="toml-editor"
            value={sectionContent}
            onChange={(e) => {
              const val = e.target.value;
              setSectionContent(val);
              setRawContent(val);
              setTomlDraftsBySection((prev) => ({ ...prev, [activeTab]: val }));
            }}
            spellCheck={false}
            placeholder={`No ${activeTab}.toml found. Content will be created on save.`}
          />
        </div>
      )}
    </>
  );
}
