import type { ComponentType } from 'react';
import type { ToolRendererProps } from './types';

import { UrlRenderer } from './UrlRenderer';
import { ShellExecRenderer } from './ShellExecRenderer';
import { ToolSearchRenderer } from './ToolSearchRenderer';
import { GlobRenderer } from './GlobRenderer';
import { GrepRenderer } from './GrepRenderer';
import { FileToolRenderer } from './FileToolRenderer';
import { EnterPlanModeRenderer } from './EnterPlanModeRenderer';
import { PlanWriterRenderer } from './PlanWriterRenderer';
import { PlanRenderer } from './PlanRenderer';
import { ExitPlanModeRenderer } from './ExitPlanModeRenderer';
import { AskUserRenderer } from './AskUserRenderer';

export type { ToolRendererProps } from './types';
export { DefaultRenderer } from './DefaultRenderer';

/**
 * Registry mapping tool names to their renderer components.
 * To add a new tool type: create a FooRenderer.tsx, then add an entry here.
 */
export const TOOL_RENDERERS: Record<string, ComponentType<ToolRendererProps>> = {
  Browser: UrlRenderer,
  WebFetch: UrlRenderer,
  ShellExec: ShellExecRenderer,
  ToolSearch: ToolSearchRenderer,
  Glob: GlobRenderer,
  Grep: GrepRenderer,
  FileEdit: FileToolRenderer,
  FileWrite: FileToolRenderer,
  FileRead: FileToolRenderer,
  Plan: PlanRenderer,
  EnterPlanMode: EnterPlanModeRenderer,
  PlanWriter: PlanWriterRenderer,
  ExitPlanMode: ExitPlanModeRenderer,
  AskUser: AskUserRenderer,
};
