// ---------------------------------------------------------------------------
// useProviders -- provider and system status management.
//
// Extracted from App.tsx. Manages:
//   - System status (version, provider count, session count)
//   - Provider list (for InputArea dropdown)
//   - Selected provider ID
//   - Provider icon map (parsed from TOML config)
// ---------------------------------------------------------------------------

import { useState, useCallback, useEffect } from 'react';
import { transport } from '../lib';
import type { SystemStatus, ProviderInfo } from '../types';

export interface UseProvidersReturn {
  systemStatus: SystemStatus | null;
  providers: ProviderInfo[];
  selectedProviderId: string;
  setSelectedProviderId: (id: string) => void;
  providerIconMap: Record<string, string>;
  refreshProviders: () => void;
  refreshProviderIcons: () => void;
}

export function useProviders(
  loadSection: (section: string) => Promise<string>,
): UseProvidersReturn {
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState('auto');
  const [providerIconMap, setProviderIconMap] = useState<Record<string, string>>({});

  // Fetch the latest provider list from backend.
  const refreshProviders = useCallback(() => {
    transport.invoke<ProviderInfo[]>('provider_list')
      .then(setProviders)
      .catch((e) => console.warn('Failed to load provider list:', e));
  }, []);

  // Build provider icon map from the providers TOML config.
  const refreshProviderIcons = useCallback(() => {
    loadSection('providers')
      .then((toml) => {
        try {
          // Simple TOML parsing: extract icon = "..." lines within [[providers]] blocks.
          const map: Record<string, string> = {};
          let currentId: string | null = null;
          for (const line of toml.split('\n')) {
            const trimmed = line.trim();
            const idMatch = trimmed.match(/^id\s*=\s*"([^"]+)"/);
            if (idMatch) {
              currentId = idMatch[1];
            }
            const iconMatch = trimmed.match(/^icon\s*=\s*"([^"]+)"/);
            if (iconMatch && currentId) {
              map[currentId] = iconMatch[1];
            }
            // Reset on new provider block.
            if (trimmed === '[[providers]]') {
              currentId = null;
            }
          }
          setProviderIconMap(map);
        } catch {
          // Ignore parse errors for icon map.
        }
      })
      .catch((e) => console.warn('Failed to load provider icons:', e));
  }, [loadSection]);

  // Load system status and provider list on mount.
  useEffect(() => {
    transport.invoke<SystemStatus>('system_status')
      .then(setSystemStatus)
      .catch((e) => console.warn('Failed to load system status:', e));
    refreshProviders();
    refreshProviderIcons();
  }, [refreshProviders, refreshProviderIcons]);

  return {
    systemStatus,
    providers,
    selectedProviderId,
    setSelectedProviderId,
    providerIconMap,
    refreshProviders,
    refreshProviderIcons,
  };
}
