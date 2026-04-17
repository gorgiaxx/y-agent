import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { McpServerFormData } from '../components/settings/settingsTypes';
import { jsonToMcpServers } from '../components/settings/settingsTypes';

export function useMcpServers() {
  const [servers, setServers] = useState<McpServerFormData[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const json = await invoke<any>('mcp_config_get');
      setServers(jsonToMcpServers(json));
    } catch {
      setServers([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { servers, loading, refresh };
}
