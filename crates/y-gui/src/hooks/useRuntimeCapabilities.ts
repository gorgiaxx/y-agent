import { useEffect, useState } from 'react';

import { transport } from '../lib';
import type { RuntimeCapabilities } from '../types';

export async function loadRuntimeCapabilities(): Promise<RuntimeCapabilities> {
  return transport.invoke<RuntimeCapabilities>('runtime_capabilities');
}

export interface RuntimeCapabilitiesState {
  capabilities: RuntimeCapabilities | null;
  loading: boolean;
  error: string | null;
}

export function useRuntimeCapabilities(): RuntimeCapabilitiesState {
  const [state, setState] = useState<RuntimeCapabilitiesState>({
    capabilities: null,
    loading: true,
    error: null,
  });

  useEffect(() => {
    let active = true;
    void loadRuntimeCapabilities()
      .then((capabilities) => {
        if (active) setState({ capabilities, loading: false, error: null });
      })
      .catch((cause: unknown) => {
        if (!active) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        setState({ capabilities: null, loading: false, error: message });
      });

    return () => {
      active = false;
    };
  }, []);

  return state;
}
