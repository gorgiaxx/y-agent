import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SkillInfo, SkillDetail } from '../types';

export function useSkills() {
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(false);

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

  return { skills, loading, refresh, getSkillDetail, uninstallSkill, setEnabled, openFolder };
}
