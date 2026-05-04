import { useBackgroundTasksContext, useBackgroundTasksNavContext } from '../../providers/AppContexts';
import { BackgroundTasksSidebarPanel } from './BackgroundTasksPanel';

export function BackgroundTasksSidebarContent() {
  const backgroundTasks = useBackgroundTasksContext();
  const nav = useBackgroundTasksNavContext();
  const {
    tasks,
    loading,
    error,
    refresh,
  } = backgroundTasks;
  const selectedTask = tasks.find(
    (task) => task.process_id === nav.selectedBackgroundTaskId,
  ) ?? tasks[0] ?? null;

  return (
    <BackgroundTasksSidebarPanel
      tasks={tasks}
      loading={loading}
      error={error}
      selectedProcessId={selectedTask?.process_id ?? null}
      onSelectTask={nav.setSelectedBackgroundTaskId}
      onRefresh={() => void refresh()}
    />
  );
}
