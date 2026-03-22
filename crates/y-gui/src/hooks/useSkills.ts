import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SkillInfo, SkillDetail, SkillFileEntry, SkillImportResult } from '../types';

export type ImportStatus = 'idle' | 'importing' | 'success' | 'error';

export function useSkills() {
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [importStatus, setImportStatus] = useState<ImportStatus>('idle');
  const [importError, setImportError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<SkillInfo[]>('skill_list');
      setSkills(list);
    } catch (err) {
      console.error('Failed to load skills:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const getSkillDetail = useCallback(async (name: string): Promise<SkillDetail | null> => {
    try {
      return await invoke<SkillDetail>('skill_get', { name });
    } catch (err) {
      console.error('Failed to get skill detail:', err);
      return null;
    }
  }, []);

  const uninstallSkill = useCallback(async (name: string) => {
    try {
      await invoke('skill_uninstall', { name });
      await refresh();
    } catch (err) {
      console.error('Failed to uninstall skill:', err);
    }
  }, [refresh]);

  const setEnabled = useCallback(async (name: string, enabled: boolean) => {
    try {
      await invoke('skill_set_enabled', { name, enabled });
      await refresh();
    } catch (err) {
      console.error('Failed to toggle skill:', err);
    }
  }, [refresh]);

  const openFolder = useCallback(async (name: string) => {
    try {
      await invoke('skill_open_folder', { name });
    } catch (err) {
      console.error('Failed to open folder:', err);
    }
  }, []);

  /** Fire-and-forget import: updates importStatus/importError in background. */
  const importSkill = useCallback((path: string, sanitize: boolean) => {
    setImportStatus('importing');
    setImportError(null);

    invoke<SkillImportResult>('skill_import', { path, sanitize })
      .then(async (result) => {
        if (result.decision === 'accepted' || result.decision === 'optimized') {
          setImportStatus('success');
          await refresh();
        } else if (result.decision === 'rejected') {
          setImportStatus('error');
          setImportError(result.error || 'Skill was rejected');
        } else {
          // partial_accept
          setImportStatus('success');
          await refresh();
        }
      })
      .catch((err) => {
        setImportStatus('error');
        setImportError(String(err));
        console.error('Failed to import skill:', err);
      });
  }, [refresh]);

  const clearImportStatus = useCallback(() => {
    setImportStatus('idle');
    setImportError(null);
  }, []);

  const getSkillFiles = useCallback(async (name: string): Promise<SkillFileEntry[]> => {
    try {
      return await invoke<SkillFileEntry[]>('skill_get_files', { name });
    } catch (err) {
      console.error('Failed to get skill files:', err);
      return [];
    }
  }, []);

  const readSkillFile = useCallback(async (name: string, relativePath: string): Promise<string | null> => {
    try {
      return await invoke<string>('skill_read_file', { name, relativePath });
    } catch (err) {
      console.error('Failed to read skill file:', err);
      return null;
    }
  }, []);

  const saveSkillFile = useCallback(async (name: string, relativePath: string, content: string): Promise<boolean> => {
    try {
      await invoke('skill_save_file', { name, relativePath, content });
      return true;
    } catch (err) {
      console.error('Failed to save skill file:', err);
      return false;
    }
  }, []);

  return {
    skills,
    loading,
    refresh,
    getSkillDetail,
    uninstallSkill,
    setEnabled,
    openFolder,
    importSkill,
    importStatus,
    importError,
    clearImportStatus,
    getSkillFiles,
    readSkillFile,
    saveSkillFile,
  };
}
