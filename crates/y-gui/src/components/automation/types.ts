import type {
  WorkflowInfo,
  ScheduleInfo,
  DagVisualization,
  ValidationResult,
  ExecutionRecord,
} from '../../hooks/useAutomation';

export type { WorkflowInfo, ScheduleInfo, DagVisualization, ValidationResult, ExecutionRecord };

export interface AutomationPanelProps {
  selectedType: 'workflow' | 'schedule' | null;
  selectedId: string | null;
  // Workflow ops
  getWorkflow: (id: string) => Promise<WorkflowInfo | null>;
  createWorkflow: (
    name: string,
    definition: string,
    format: string,
    description?: string,
    tags?: string,
  ) => Promise<WorkflowInfo | null>;
  updateWorkflow: (
    id: string,
    definition?: string,
    format?: string,
    description?: string,
    tags?: string,
  ) => Promise<WorkflowInfo | null>;
  deleteWorkflow: (id: string) => Promise<boolean>;
  validateWorkflow: (definition: string, format: string) => Promise<ValidationResult | null>;
  getWorkflowDag: (id: string) => Promise<DagVisualization | null>;
  // Schedule ops
  schedules: ScheduleInfo[];
  workflows: WorkflowInfo[];
  getSchedule: (id: string) => Promise<ScheduleInfo | null>;
  createSchedule: (request: {
    name: string;
    trigger: unknown;
    workflow_id: string;
    description?: string;
  }) => Promise<ScheduleInfo | null>;
  updateSchedule: (id: string, request: {
    name?: string;
    trigger?: unknown;
    workflow_id?: string;
    description?: string;
  }) => Promise<ScheduleInfo | null>;
  deleteSchedule: (id: string) => Promise<boolean>;
  pauseSchedule: (id: string) => Promise<boolean>;
  resumeSchedule: (id: string) => Promise<boolean>;
  // Execution history & replay
  getExecutionHistory: (scheduleId: string) => Promise<ExecutionRecord[]>;
  triggerScheduleNow: (scheduleId: string) => Promise<ExecutionRecord | null>;
  executeWorkflow: (workflowId: string) => Promise<ExecutionRecord | null>;
  // Creating new
  isCreating: 'workflow' | 'schedule' | null;
  onCancelCreate: () => void;
}
