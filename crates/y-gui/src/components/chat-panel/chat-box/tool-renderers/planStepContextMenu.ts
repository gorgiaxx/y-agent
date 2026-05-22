import type { PlanTaskDisplay } from '../planToolDisplay';

export interface PlanStepMenuOptions {
  planRunId: string;
  sessionId: string;
  task: PlanTaskDisplay;
  onRetryFromHere: (planRunId: string, taskId: string) => void;
}

export async function showPlanStepContextMenu(opts: PlanStepMenuOptions): Promise<void> {
  const { planRunId, task, onRetryFromHere } = opts;

  if (!planRunId) return;

  try {
    const { Menu } = await import('@tauri-apps/api/menu');
    const menu = await Menu.new({
      items: [
        {
          text: `Retry from: ${task.title}`,
          action: () => onRetryFromHere(planRunId, task.id),
        },
      ],
    });
    await menu.popup();
  } catch {
    // Not in Tauri environment; silently no-op.
  }
}
