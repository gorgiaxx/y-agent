import { useEffect, useMemo, useRef } from 'react';

import { BackgroundTasksOutputPanel } from '../components/background-tasks/BackgroundTasksPanel';
import { useBackgroundTasksContext, useBackgroundTasksNavContext } from '../providers/AppContexts';

export function BackgroundTasksView() {
  const backgroundTasks = useBackgroundTasksContext();
  const nav = useBackgroundTasksNavContext();
  const {
    tasks,
    logs,
    busyProcessId,
    refresh,
    pollTask,
    killTask,
  } = backgroundTasks;
  const { selectedBackgroundTaskId, setSelectedBackgroundTaskId } = nav;
  const tasksRef = useRef(tasks);

  useEffect(() => {
    tasksRef.current = tasks;
  }, [tasks]);

  useEffect(() => {
    const selectedStillExists = tasks.some(
      (task) => task.process_id === selectedBackgroundTaskId,
    );
    if (!selectedStillExists) {
      setSelectedBackgroundTaskId(tasks[0]?.process_id ?? null);
    }
  }, [selectedBackgroundTaskId, setSelectedBackgroundTaskId, tasks]);

  useEffect(() => {
    const pollRunningTasks = () => {
      for (const task of tasksRef.current) {
        if (task.status === 'running') {
          void pollTask(task.process_id);
        }
      }
    };

    void refresh();
    pollRunningTasks();
    const id = window.setInterval(pollRunningTasks, 1_500);
    return () => window.clearInterval(id);
  }, [pollTask, refresh]);

  const selectedTask = useMemo(
    () => tasks.find((task) => task.process_id === selectedBackgroundTaskId) ?? null,
    [selectedBackgroundTaskId, tasks],
  );

  return (
    <BackgroundTasksOutputPanel
      task={selectedTask}
      logs={selectedTask ? logs[selectedTask.process_id] ?? [] : []}
      busy={selectedTask ? busyProcessId === selectedTask.process_id : false}
      onPoll={(processId) => void pollTask(processId)}
      onKill={(processId) => void killTask(processId)}
    />
  );
}
