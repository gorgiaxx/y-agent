export interface PlanRequestMeta {
  request: string;
  context: string;
}

export interface PlanTaskDisplay {
  id: string;
  phase: number;
  title: string;
  description: string;
  dependsOn: string[];
  status: string;
  estimatedIterations: number;
  keyFiles: string[];
  acceptanceCriteria: string[];
}

export interface PlanWriterStageDisplay {
  kind: 'plan_stage';
  stage: 'plan_writer';
  stageStatus: string;
  planTitle: string;
  planFile: string;
  planContent: string;
  estimatedEffort: string;
  overview: string;
  scopeIn: string[];
  scopeOut: string[];
  guardrails: string[];
  reviewStatus: string;
  reviewFeedback: string;
  tasks: PlanTaskDisplay[];
}

export interface PlanExecutionDisplay {
  kind: 'plan_execution';
  planTitle: string;
  planFile: string;
  totalPhases: number;
  completed: number;
  failed: number;
  tasks: PlanTaskDisplay[];
  phases: Array<Record<string, unknown>>;
}

export type PlanDisplayMeta =
  | PlanWriterStageDisplay
  | PlanExecutionDisplay;

function tryParseJson(raw: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === 'object' && parsed !== null
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object'
    ? value as Record<string, unknown>
    : null;
}

export function extractPlanRequestMeta(argsRaw: string): PlanRequestMeta | null {
  const obj = tryParseJson(argsRaw);
  if (!obj) return null;
  const request = typeof obj.request === 'string' ? obj.request : '';
  if (!request) return null;
  return {
    request,
    context: typeof obj.context === 'string' ? obj.context : '',
  };
}

function parsePlanTask(value: unknown): PlanTaskDisplay | null {
  const obj = asObject(value);
  if (!obj) return null;
  const title = typeof obj.title === 'string' ? obj.title
    : typeof obj.label === 'string' ? obj.label
      : '';
  if (!title) return null;
  return {
    id: typeof obj.id === 'string' ? obj.id : '',
    phase: typeof obj.phase === 'number' ? obj.phase : 0,
    title,
    description: typeof obj.description === 'string' ? obj.description : '',
    dependsOn: Array.isArray(obj.depends_on)
      ? obj.depends_on.map((dep) => String(dep))
      : [],
    status: typeof obj.status === 'string' ? obj.status : 'pending',
    estimatedIterations: typeof obj.estimated_iterations === 'number'
      ? obj.estimated_iterations
      : typeof obj.iterations === 'number'
        ? obj.iterations
        : 0,
    keyFiles: Array.isArray(obj.key_files)
      ? obj.key_files.map((file) => String(file))
      : [],
    acceptanceCriteria: Array.isArray(obj.acceptance_criteria)
      ? obj.acceptance_criteria.map((item) => String(item))
      : [],
  };
}

function parseStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.map((item) => String(item)) : [];
}

function mergeExecutionTaskStatuses(
  tasks: PlanTaskDisplay[],
  phases: Array<Record<string, unknown>>,
): PlanTaskDisplay[] {
  if (tasks.length === 0 || phases.length === 0) return tasks;

  const statusByTaskId = new Map<string, string>();
  const statusByPhase = new Map<number, string>();
  const statusByTitle = new Map<string, string>();

  for (const phase of phases) {
    const status = typeof phase.status === 'string' ? phase.status : '';
    if (!status) continue;

    if (typeof phase.task_id === 'string' && phase.task_id) {
      statusByTaskId.set(phase.task_id, status);
    }
    if (typeof phase.phase === 'number') {
      statusByPhase.set(phase.phase, status);
    }
    if (typeof phase.title === 'string' && phase.title) {
      statusByTitle.set(phase.title, status);
    }
  }

  return tasks.map((task) => ({
    ...task,
    status: statusByTaskId.get(task.id)
      ?? statusByPhase.get(task.phase)
      ?? statusByTitle.get(task.title)
      ?? task.status,
  }));
}

function parsePlanDisplayObject(obj: Record<string, unknown>): PlanDisplayMeta | null {
  const kind = typeof obj.kind === 'string' ? obj.kind : '';

  if (kind === 'plan_stage') {
    const stage = typeof obj.stage === 'string' ? obj.stage : '';
    const stageStatus = typeof obj.stage_status === 'string' ? obj.stage_status : 'completed';
    const planTitle = typeof obj.plan_title === 'string' ? obj.plan_title : '';
    const planFile = typeof obj.plan_file === 'string' ? obj.plan_file : '';

    if (stage === 'plan_writer') {
      const tasks = Array.isArray(obj.tasks)
        ? obj.tasks.map(parsePlanTask).filter((task): task is PlanTaskDisplay => task != null)
        : [];
      return {
        kind: 'plan_stage',
        stage,
        stageStatus,
        planTitle,
        planFile,
        planContent: typeof obj.plan_content === 'string' ? obj.plan_content : '',
        estimatedEffort: typeof obj.estimated_effort === 'string' ? obj.estimated_effort : '',
        overview: typeof obj.overview === 'string' ? obj.overview : '',
        scopeIn: parseStringArray(obj.scope_in),
        scopeOut: parseStringArray(obj.scope_out),
        guardrails: parseStringArray(obj.guardrails),
        reviewStatus: typeof obj.review_status === 'string' ? obj.review_status : '',
        reviewFeedback: typeof obj.review_feedback === 'string' ? obj.review_feedback : '',
        tasks,
      };
    }
  }

  if (kind === 'plan_execution') {
    const hasPlanFields = typeof obj.plan_title === 'string'
      || typeof obj.plan_file === 'string'
      || typeof obj.total_phases === 'number';
    if (!hasPlanFields) return null;

    const tasks = Array.isArray(obj.tasks)
      ? obj.tasks.map(parsePlanTask).filter((task): task is PlanTaskDisplay => task != null)
      : [];
    const phases = Array.isArray(obj.phases)
      ? obj.phases.filter((phase): phase is Record<string, unknown> => (
        phase != null && typeof phase === 'object'
      ))
      : [];
    const mergedTasks = mergeExecutionTaskStatuses(tasks, phases);

    return {
      kind: 'plan_execution',
      planTitle: typeof obj.plan_title === 'string' ? obj.plan_title : '',
      planFile: typeof obj.plan_file === 'string' ? obj.plan_file : '',
      totalPhases: typeof obj.total_phases === 'number' ? obj.total_phases : tasks.length,
      completed: typeof obj.completed === 'number' ? obj.completed : 0,
      failed: typeof obj.failed === 'number' ? obj.failed : 0,
      tasks: mergedTasks,
      phases,
    };
  }

  return null;
}

export function extractPlanDisplayMeta(
  metadata: unknown,
  resultRaw?: string,
): PlanDisplayMeta | null {
  const metaObj = asObject(metadata);
  const displayObj = asObject(metaObj?.display);
  if (displayObj) {
    const display = parsePlanDisplayObject(displayObj);
    if (display) return display;
  }

  const resultObj = resultRaw ? tryParseJson(resultRaw) : null;
  if (resultObj) {
    return parsePlanDisplayObject({
      kind: 'plan_execution',
      ...resultObj,
    });
  }

  return null;
}
