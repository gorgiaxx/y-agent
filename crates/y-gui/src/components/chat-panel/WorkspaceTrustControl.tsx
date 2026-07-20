import { useEffect, useState } from 'react';

import { getWorkspaceTrust, setWorkspaceTrust } from '../../hooks/useWorkspaces';
import type { WorkspaceTrustDecision, WorkspaceTrustStatus } from '../../types';
import { Button } from '../ui';

interface WorkspaceTrustSectionProps {
  decision: WorkspaceTrustDecision | null;
  loading: boolean;
  error: string | null;
  busy: boolean;
  onSetTrust: (trusted: boolean) => void;
}

const STATUS_LABELS: Record<WorkspaceTrustStatus, string> = {
  unknown: 'Unknown',
  trusted: 'Trusted',
  untrusted: 'Blocked',
};

export function WorkspaceTrustSection({
  decision,
  loading,
  error,
  busy,
  onSetTrust,
}: WorkspaceTrustSectionProps) {
  const status = decision?.status ?? 'unknown';

  return (
    <section className="flex flex-col gap-2 rounded-md border border-solid border-[var(--border)] bg-[var(--surface-secondary)] p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="text-11px font-600 text-[var(--text-primary)]">Project configuration trust</div>
        <span className="text-10px font-600 uppercase tracking-[0.04em] text-[var(--text-muted)]">
          {loading ? 'Checking' : STATUS_LABELS[status]}
        </span>
      </div>

      <p className="m-0 text-11px leading-4 text-[var(--text-secondary)]">
        Trust allows this folder&apos;s y-agent.toml and config/*.toml to become active. It does not bypass tool permissions or HITL approval.
      </p>

      {decision && (
        <div className="truncate font-mono text-10px text-[var(--text-muted)]" title={decision.canonical_path}>
          Canonical path: {decision.canonical_path}
        </div>
      )}

      {error && (
        <div className="text-10px text-[var(--error)]" role="alert">
          {error}
        </div>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        {status !== 'trusted' && (
          <Button
            type="button"
            size="sm"
            variant="primary"
            disabled={loading || busy || !!error}
            onClick={() => onSetTrust(true)}
          >
            Trust project config
          </Button>
        )}
        {status !== 'untrusted' && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={loading || busy || !!error}
            onClick={() => onSetTrust(false)}
          >
            Block project config
          </Button>
        )}
      </div>
    </section>
  );
}

interface WorkspaceTrustControlProps {
  path: string;
}

interface LoadedTrustState {
  path: string;
  decision: WorkspaceTrustDecision | null;
  error: string | null;
}

export function WorkspaceTrustControl({ path }: WorkspaceTrustControlProps) {
  const normalizedPath = path.trim();
  const [loaded, setLoaded] = useState<LoadedTrustState>({
    path: '',
    decision: null,
    error: null,
  });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!normalizedPath) return undefined;

    let active = true;
    const timeout = window.setTimeout(() => {
      void getWorkspaceTrust(normalizedPath)
        .then((decision) => {
          if (active) setLoaded({ path: normalizedPath, decision, error: null });
        })
        .catch((cause: unknown) => {
          if (active) {
            const message = cause instanceof Error ? cause.message : String(cause);
            setLoaded({ path: normalizedPath, decision: null, error: message });
          }
        });
    }, 250);

    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [normalizedPath]);

  if (!normalizedPath) return null;

  const loading = loaded.path !== normalizedPath;
  const decision = loading ? null : loaded.decision;
  const error = loading ? null : loaded.error;

  const handleSetTrust = async (trusted: boolean) => {
    setBusy(true);
    try {
      const next = await setWorkspaceTrust(normalizedPath, trusted);
      setLoaded({ path: normalizedPath, decision: next, error: null });
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setLoaded({ path: normalizedPath, decision: null, error: message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <WorkspaceTrustSection
      decision={decision}
      loading={loading}
      error={error}
      busy={busy}
      onSetTrust={(trusted) => { void handleSetTrust(trusted); }}
    />
  );
}
