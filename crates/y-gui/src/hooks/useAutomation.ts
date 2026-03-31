/**
 * useAutomation -- React hook for workflow and schedule management.
 *
 * Wraps Tauri `invoke` calls to the automation command module and
 * manages local state for workflows and schedules.
 *
 * Reactivity:
 * - Auto-fetches on mount.
 * - Listens for `chat:complete` events to refresh after agent tool calls
 *   that may have mutated backend state.
 * - Re-fetches when `active` changes from false to true (tab activation).
 */
import { useState, useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Workflow template as returned by the backend. */
export interface WorkflowInfo {
  id: string;
  name: string;
  description: string | null;
  definition: string;
  format: string;
  compiled_dag: string;
  parameter_schema: string | null;
  tags: string;
  creator: string;
  created_at: string;
  updated_at: string;
}

/** Schedule summary as returned by the backend. */
export interface ScheduleInfo {
  id: string;
  name: string;
  enabled: boolean;
  trigger_type: string;
  trigger_value: string;
  workflow_id: string;
  description: string;
  tags: string[];
  created_at: string;
  last_fire: string | null;
}

/** DAG visualization data from the backend. */
export interface DagVisualization {
  nodes: DagNode[];
  edges: DagEdge[];
  topological_order: string[];
}

export interface DagNode {
  id: string;
  /** Backend sends `name`; older code may pass `label`. */
  name?: string;
  label?: string;
  /** Backend sends `task_type`; older code may pass `node_type`. */
  task_type?: string;
  node_type?: string;
}

export interface DagEdge {
  source: string;
  target: string;
  label: string | null;
}

/** Validation result from the backend. */
export interface ValidationResult {
  valid: boolean;
  errors: string[];
  dag: DagVisualization | null;
}

/** Execution record for schedule/workflow runs. */
export interface ExecutionRecord {
  execution_id: string;
  schedule_id: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'skipped';
  triggered_at: string;
  started_at: string | null;
  completed_at: string | null;
  duration_ms: number | null;
  workflow_execution_id: string | null;
  request_summary: Record<string, unknown>;
  response_summary: Record<string, unknown>;
  error_message: string | null;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useAutomation(active = true) {
  const [workflows, setWorkflows] = useState<WorkflowInfo[]>([]);
  const [schedules, setSchedules] = useState<ScheduleInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const prevActive = useRef(false);

  // --- Fetch all ---
  const refreshWorkflows = useCallback(async () => {
    try {
      const list = await invoke<WorkflowInfo[]>('workflow_list');
      setWorkflows(list);
    } catch (e) {
      console.error('workflow_list failed:', e);
    }
  }, []);

  const refreshSchedules = useCallback(async () => {
    try {
      const list = await invoke<ScheduleInfo[]>('schedule_list');
      setSchedules(list);
    } catch (e) {
      console.error('schedule_list failed:', e);
    }
  }, []);

  const refreshAll = useCallback(async () => {
    setLoading(true);
    await Promise.all([refreshWorkflows(), refreshSchedules()]);
    setLoading(false);
  }, [refreshWorkflows, refreshSchedules]);

  // Auto-fetch on mount.
  useEffect(() => {
    refreshAll();
  }, [refreshAll]);

  // Refresh when the automation tab becomes active (false -> true transition).
  useEffect(() => {
    if (active && !prevActive.current) {
      refreshAll();
    }
    prevActive.current = active;
  }, [active, refreshAll]);

  // Listen for chat:complete events -- agent tool calls during a chat turn
  // may have created/modified/deleted workflows or schedules.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<unknown>('chat:complete', () => {
      refreshAll();
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [refreshAll]);

  // --- Workflow operations ---
  const getWorkflow = useCallback(async (id: string): Promise<WorkflowInfo | null> => {
    try {
      return await invoke<WorkflowInfo>('workflow_get', { id });
    } catch (e) {
      console.error('workflow_get failed:', e);
      return null;
    }
  }, []);

  const createWorkflow = useCallback(async (
    name: string,
    definition: string,
    format: string,
    description?: string,
    tags?: string,
  ): Promise<WorkflowInfo | null> => {
    try {
      const result = await invoke<WorkflowInfo>('workflow_create', {
        name,
        definition,
        format,
        description: description ?? null,
        tags: tags ?? null,
      });
      await refreshWorkflows();
      return result;
    } catch (e) {
      console.error('workflow_create failed:', e);
      return null;
    }
  }, [refreshWorkflows]);

  const updateWorkflow = useCallback(async (
    id: string,
    definition?: string,
    format?: string,
    description?: string,
    tags?: string,
  ): Promise<WorkflowInfo | null> => {
    try {
      const result = await invoke<WorkflowInfo>('workflow_update', {
        id,
        definition: definition ?? null,
        format: format ?? null,
        description: description ?? null,
        tags: tags ?? null,
      });
      await refreshWorkflows();
      return result;
    } catch (e) {
      console.error('workflow_update failed:', e);
      return null;
    }
  }, [refreshWorkflows]);

  const deleteWorkflow = useCallback(async (id: string): Promise<boolean> => {
    try {
      const deleted = await invoke<boolean>('workflow_delete', { id });
      if (deleted) await refreshWorkflows();
      return deleted;
    } catch (e) {
      console.error('workflow_delete failed:', e);
      return false;
    }
  }, [refreshWorkflows]);

  const validateWorkflow = useCallback(async (
    definition: string,
    format: string,
  ): Promise<ValidationResult | null> => {
    try {
      return await invoke<ValidationResult>('workflow_validate', { definition, format });
    } catch (e) {
      console.error('workflow_validate failed:', e);
      return null;
    }
  }, []);

  const getWorkflowDag = useCallback(async (id: string): Promise<DagVisualization | null> => {
    try {
      return await invoke<DagVisualization>('workflow_dag', { id });
    } catch (e) {
      console.error('workflow_dag failed:', e);
      return null;
    }
  }, []);

  // --- Schedule operations ---
  const getSchedule = useCallback(async (id: string): Promise<ScheduleInfo | null> => {
    try {
      return await invoke<ScheduleInfo>('schedule_get', { id });
    } catch (e) {
      console.error('schedule_get failed:', e);
      return null;
    }
  }, []);

  const createSchedule = useCallback(async (request: {
    name: string;
    trigger: unknown;
    workflow_id: string;
    parameter_values?: unknown;
    description?: string;
    tags?: string[];
  }): Promise<ScheduleInfo | null> => {
    try {
      const result = await invoke<ScheduleInfo>('schedule_create', { request });
      await refreshSchedules();
      return result;
    } catch (e) {
      console.error('schedule_create failed:', e);
      return null;
    }
  }, [refreshSchedules]);

  const deleteSchedule = useCallback(async (id: string): Promise<boolean> => {
    try {
      const deleted = await invoke<boolean>('schedule_delete', { id });
      if (deleted) await refreshSchedules();
      return deleted;
    } catch (e) {
      console.error('schedule_delete failed:', e);
      return false;
    }
  }, [refreshSchedules]);

  const pauseSchedule = useCallback(async (id: string): Promise<boolean> => {
    try {
      await invoke('schedule_pause', { id });
      await refreshSchedules();
      return true;
    } catch (e) {
      console.error('schedule_pause failed:', e);
      return false;
    }
  }, [refreshSchedules]);

  const resumeSchedule = useCallback(async (id: string): Promise<boolean> => {
    try {
      await invoke('schedule_resume', { id });
      await refreshSchedules();
      return true;
    } catch (e) {
      console.error('schedule_resume failed:', e);
      return false;
    }
  }, [refreshSchedules]);

  const updateSchedule = useCallback(async (
    id: string,
    request: {
      name?: string;
      trigger?: unknown;
      workflow_id?: string;
      description?: string;
    },
  ): Promise<ScheduleInfo | null> => {
    try {
      const result = await invoke<ScheduleInfo>('schedule_update', { id, request });
      await refreshSchedules();
      return result;
    } catch (e) {
      console.error('schedule_update failed:', e);
      return null;
    }
  }, [refreshSchedules]);

  // --- Execution history & replay operations ---
  const getExecutionHistory = useCallback(async (
    scheduleId: string,
  ): Promise<ExecutionRecord[]> => {
    try {
      return await invoke<ExecutionRecord[]>('schedule_execution_history', {
        scheduleId,
      });
    } catch (e) {
      console.error('schedule_execution_history failed:', e);
      return [];
    }
  }, []);

  const getExecution = useCallback(async (
    executionId: string,
  ): Promise<ExecutionRecord | null> => {
    try {
      return await invoke<ExecutionRecord>('schedule_execution_get', {
        executionId,
      });
    } catch (e) {
      console.error('schedule_execution_get failed:', e);
      return null;
    }
  }, []);

  const triggerScheduleNow = useCallback(async (
    scheduleId: string,
  ): Promise<ExecutionRecord | null> => {
    try {
      return await invoke<ExecutionRecord>('schedule_trigger_now', {
        scheduleId,
      });
    } catch (e) {
      console.error('schedule_trigger_now failed:', e);
      return null;
    }
  }, []);

  const executeWorkflow = useCallback(async (
    workflowId: string,
  ): Promise<ExecutionRecord | null> => {
    try {
      return await invoke<ExecutionRecord>('workflow_execute', {
        workflowId,
      });
    } catch (e) {
      console.error('workflow_execute failed:', e);
      return null;
    }
  }, []);

  return {
    workflows,
    schedules,
    loading,
    refreshAll,
    // Workflow ops
    getWorkflow,
    createWorkflow,
    updateWorkflow,
    deleteWorkflow,
    validateWorkflow,
    getWorkflowDag,
    // Schedule ops
    getSchedule,
    createSchedule,
    updateSchedule,
    deleteSchedule,
    pauseSchedule,
    resumeSchedule,
    // Execution history & replay
    getExecutionHistory,
    getExecution,
    triggerScheduleNow,
    executeWorkflow,
  };
}
