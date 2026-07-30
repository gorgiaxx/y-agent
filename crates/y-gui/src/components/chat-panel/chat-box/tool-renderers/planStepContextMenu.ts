import { platform } from '../../../../lib';
import type { ContextMenuItem } from '../../../../lib/platform';
import type { PlanTaskDisplay } from '../planToolDisplay';

export interface PlanStepMenuOptions {
  planRunId: string;
  sessionId: string;
  task: PlanTaskDisplay;
  onRetryFromHere: (planRunId: string, taskId: string) => void;
}

export function buildPlanStepContextMenuItems(
  opts: PlanStepMenuOptions,
): ContextMenuItem[] {
  const { planRunId, task, onRetryFromHere } = opts;
  if (!planRunId) return [];

  return [{
    kind: 'item',
    text: `Retry from: ${task.title}`,
    action: () => onRetryFromHere(planRunId, task.id),
  }];
}

export async function showPlanStepContextMenu(opts: PlanStepMenuOptions): Promise<void> {
  if (!platform.capabilities.nativeContextMenus) return;
  const items = buildPlanStepContextMenuItems(opts);
  if (items.length > 0) await platform.showContextMenu(items);
}
