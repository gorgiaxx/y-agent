import { useState, useEffect } from 'react';
import { getConnectionStatus, onConnectionStatusChange } from '../lib';
import type { ConnectionStatus } from '../lib';

export function useConnectionStatus(): ConnectionStatus {
  const [status, setStatus] = useState<ConnectionStatus>(getConnectionStatus);

  useEffect(() => {
    return onConnectionStatusChange(setStatus);
  }, []);

  return status;
}
