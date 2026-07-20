import type { RuntimeFeatureAvailability } from '../../types';

interface FeatureAvailabilityNoticeProps {
  featureName: string;
  availability: RuntimeFeatureAvailability | null | undefined;
  error?: string | null;
  plural?: boolean;
}

export function FeatureAvailabilityNotice({
  featureName,
  availability,
  error,
  plural = false,
}: FeatureAvailabilityNoticeProps) {
  if (error) {
    return (
      <div
        role="alert"
        className="rounded-md border border-solid border-[var(--error)] bg-[var(--surface-secondary)] p-3 text-11px text-[var(--text-secondary)]"
      >
        {featureName} availability could not be confirmed, so these controls remain read-only. {error}
      </div>
    );
  }

  if (!availability) {
    return (
      <div className="rounded-md border border-solid border-[var(--border)] bg-[var(--surface-secondary)] p-3 text-11px text-[var(--text-muted)]">
        Checking whether {featureName} is available in this binary.
      </div>
    );
  }

  if (!availability.available) {
    return (
      <div className="rounded-md border border-solid border-[var(--warning)] bg-[var(--surface-secondary)] p-3 text-11px text-[var(--text-secondary)]">
        {featureName} {plural ? 'are' : 'is'} not compiled into this binary. These controls are read-only and saved values will remain inactive.
      </div>
    );
  }

  if (availability.restart_required) {
    return (
      <div className="rounded-md border border-solid border-[var(--border)] bg-[var(--surface-secondary)] p-3 text-11px text-[var(--text-secondary)]">
        Changes to {featureName} are saved immediately but require an application restart to become active.
      </div>
    );
  }

  return null;
}
