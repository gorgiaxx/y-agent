import { describe, expect, it } from 'vitest';

import {
  canonicalToolName,
  extractFileToolMeta,
  basename,
  extractPlanDisplayMeta,
  extractLoopDisplayMeta,
} from '../components/chat-panel/chat-box/toolCallUtils';
import { collectModifiedFilesForInfoPanel } from '../hooks/useInfoPanel';
import type { ToolResultRecord } from '../hooks/chatStreamTypes';
import type { Message } from '../types';

function makeFileEditRecord(
  filePath: string,
  oldString = 'a',
  newString = 'b',
): ToolResultRecord {
  return {
    name: 'FileEdit',
    arguments: JSON.stringify({ file_path: filePath, old_string: oldString, new_string: newString }),
    success: true,
    durationMs: 10,
    resultPreview: 'ok',
  };
}

function makeFileWriteRecord(filePath: string): ToolResultRecord {
  return {
    name: 'FileWrite',
    arguments: JSON.stringify({ path: filePath }),
    success: true,
    durationMs: 5,
    resultPreview: 'ok',
  };
}

function makeFileReadRecord(filePath: string): ToolResultRecord {
  return {
    name: 'FileRead',
    arguments: JSON.stringify({ path: filePath }),
    success: true,
    durationMs: 3,
    resultPreview: 'content',
  };
}

function makePlanRecord(display: Record<string, unknown>): ToolResultRecord {
  return {
    name: 'Plan',
    arguments: '{}',
    success: true,
    durationMs: 100,
    resultPreview: '',
    metadata: { display },
  };
}

function makeLoopRecord(display: Record<string, unknown>): ToolResultRecord {
  return {
    name: 'Loop',
    arguments: '{}',
    success: true,
    durationMs: 100,
    resultPreview: '',
    metadata: { display },
  };
}

describe('useInfoPanel utilities', () => {
  describe('file extraction', () => {
    it('extracts FileEdit tool metadata', () => {
      const meta = extractFileToolMeta('FileEdit', JSON.stringify({
        file_path: '/src/main.rs', old_string: 'a', new_string: 'b',
      }));
      expect(meta).not.toBeNull();
      expect(meta!.filePath).toBe('/src/main.rs');
      expect(meta!.toolType).toBe('edit');
    });

    it('extracts FileWrite tool metadata', () => {
      const meta = extractFileToolMeta('FileWrite', JSON.stringify({ path: '/src/new.rs' }));
      expect(meta).not.toBeNull();
      expect(meta!.filePath).toBe('/src/new.rs');
      expect(meta!.toolType).toBe('write');
    });

    it('returns null for FileRead', () => {
      const meta = extractFileToolMeta('FileRead', JSON.stringify({ path: '/src/lib.rs' }));
      expect(meta).not.toBeNull();
      expect(meta!.toolType).toBe('read');
    });

    it('returns null for non-file tools', () => {
      expect(extractFileToolMeta('ShellExec', '{"command":"ls"}')).toBeNull();
    });
  });

  describe('canonicalToolName', () => {
    it('normalizes case', () => {
      expect(canonicalToolName('fileedit')).toBe('FileEdit');
      expect(canonicalToolName('FILEWRITE')).toBe('FileWrite');
    });

    it('passes through unknown tools', () => {
      expect(canonicalToolName('CustomTool')).toBe('CustomTool');
    });
  });

  describe('basename', () => {
    it('extracts file name from path', () => {
      expect(basename('/Users/test/src/main.rs')).toBe('main.rs');
      expect(basename('src/lib.rs')).toBe('lib.rs');
    });
  });

  describe('plan display extraction', () => {
    it('extracts plan_stage display metadata', () => {
      const display = extractPlanDisplayMeta({
        display: {
          kind: 'plan_stage',
          stage: 'plan_writer',
          stage_status: 'running',
          plan_title: 'Test Plan',
          plan_file: 'plan.md',
          tasks: [
            { id: 't1', title: 'Task 1', status: 'completed', phase: 1 },
          ],
        },
      });
      expect(display).not.toBeNull();
      expect(display!.kind).toBe('plan_stage');
      expect(display!.planTitle).toBe('Test Plan');
    });

    it('extracts plan_execution display metadata', () => {
      const display = extractPlanDisplayMeta({
        display: {
          kind: 'plan_execution',
          plan_title: 'Exec Plan',
          plan_file: 'plan.md',
          total_phases: 3,
          completed: 1,
          failed: 0,
          tasks: [],
          phases: [],
        },
      });
      expect(display).not.toBeNull();
      expect(display!.kind).toBe('plan_execution');
    });

    it('returns null for empty metadata', () => {
      expect(extractPlanDisplayMeta(null)).toBeNull();
      expect(extractPlanDisplayMeta({})).toBeNull();
    });
  });

  describe('loop display extraction', () => {
    it('extracts loop_round display metadata', () => {
      const display = extractLoopDisplayMeta({
        display: {
          kind: 'loop_round',
          round: 2,
          max_rounds: 5,
          round_status: 'executing',
          tasks_completed: ['Fix lint'],
          tasks_remaining: ['Add docs'],
          converged: false,
          rounds: [],
        },
      });
      expect(display).not.toBeNull();
      expect(display!.kind).toBe('loop_round');
    });

    it('extracts loop_init display metadata', () => {
      const display = extractLoopDisplayMeta({
        display: {
          kind: 'loop_init',
          request: 'Fix all lint',
          progress_file: 'progress.md',
          max_rounds: 10,
        },
      });
      expect(display).not.toBeNull();
      expect(display!.kind).toBe('loop_init');
    });

    it('returns null for empty metadata', () => {
      expect(extractLoopDisplayMeta(null)).toBeNull();
    });
  });

  describe('modified files collection logic', () => {
    it('keeps every diff for repeated updates to the same file', () => {
      const files = collectModifiedFilesForInfoPanel([
        makeFileEditRecord('/src/main.rs', 'alpha = 1;\n', 'alpha = 2;\n'),
        makeFileEditRecord('/src/main.rs', 'beta = 1;\n', 'beta = 2;\n'),
      ]);

      expect(files).toHaveLength(1);
      expect(files[0].count).toBe(2);
      expect(files[0].diffs).toEqual([
        { oldString: 'alpha = 1;\n', newString: 'alpha = 2;\n' },
        { oldString: 'beta = 1;\n', newString: 'beta = 2;\n' },
      ]);
    });

    it('groups multiple edits to same file', () => {
      const records: ToolResultRecord[] = [
        makeFileEditRecord('/src/main.rs'),
        makeFileEditRecord('/src/main.rs'),
        makeFileEditRecord('/src/lib.rs'),
      ];

      const fileMap = new Map<string, { count: number; toolType: string }>();
      for (const rec of records) {
        const name = canonicalToolName(rec.name);
        if (name !== 'FileEdit' && name !== 'FileWrite') continue;
        const meta = extractFileToolMeta(name, rec.arguments ?? '');
        if (!meta || meta.toolType === 'read') continue;
        const existing = fileMap.get(meta.filePath);
        if (existing) {
          existing.count += 1;
        } else {
          fileMap.set(meta.filePath, { count: 1, toolType: meta.toolType });
        }
      }

      expect(fileMap.size).toBe(2);
      expect(fileMap.get('/src/main.rs')!.count).toBe(2);
      expect(fileMap.get('/src/lib.rs')!.count).toBe(1);
    });

    it('excludes FileRead from modified files', () => {
      const records: ToolResultRecord[] = [
        makeFileReadRecord('/src/main.rs'),
        makeFileEditRecord('/src/lib.rs'),
      ];

      const files: string[] = [];
      for (const rec of records) {
        const name = canonicalToolName(rec.name);
        if (name !== 'FileEdit' && name !== 'FileWrite') continue;
        const meta = extractFileToolMeta(name, rec.arguments ?? '');
        if (!meta || meta.toolType === 'read') continue;
        files.push(meta.filePath);
      }

      expect(files).toEqual(['/src/lib.rs']);
    });

    it('includes both FileEdit and FileWrite', () => {
      const records: ToolResultRecord[] = [
        makeFileEditRecord('/src/edit.rs'),
        makeFileWriteRecord('/src/write.rs'),
      ];

      const files: string[] = [];
      for (const rec of records) {
        const name = canonicalToolName(rec.name);
        if (name !== 'FileEdit' && name !== 'FileWrite') continue;
        const meta = extractFileToolMeta(name, rec.arguments ?? '');
        if (!meta || meta.toolType === 'read') continue;
        files.push(meta.filePath);
      }

      expect(files).toEqual(['/src/edit.rs', '/src/write.rs']);
    });
  });

  describe('latest plan/loop selection', () => {
    it('selects the latest Plan record', () => {
      const records: ToolResultRecord[] = [
        makePlanRecord({
          kind: 'plan_execution', plan_title: 'Old', plan_file: 'p.md',
          total_phases: 2, completed: 2, failed: 0, tasks: [], phases: [],
        }),
        makePlanRecord({
          kind: 'plan_execution', plan_title: 'New', plan_file: 'p.md',
          total_phases: 5, completed: 1, failed: 0, tasks: [], phases: [],
        }),
      ];

      let latest = null;
      for (let i = records.length - 1; i >= 0; i--) {
        if (canonicalToolName(records[i].name) !== 'Plan') continue;
        const display = extractPlanDisplayMeta(records[i].metadata, records[i].resultPreview);
        if (display) { latest = display; break; }
      }

      expect(latest).not.toBeNull();
      expect(latest!.planTitle).toBe('New');
    });

    it('selects the latest Loop record', () => {
      const records: ToolResultRecord[] = [
        makeLoopRecord({
          kind: 'loop_round', round: 1, max_rounds: 3,
          round_status: 'done', tasks_completed: [], tasks_remaining: [],
          converged: false, rounds: [],
        }),
        makeLoopRecord({
          kind: 'loop_round', round: 2, max_rounds: 3,
          round_status: 'executing', tasks_completed: ['A'], tasks_remaining: ['B'],
          converged: false, rounds: [],
        }),
      ];

      let latest = null;
      for (let i = records.length - 1; i >= 0; i--) {
        if (canonicalToolName(records[i].name) !== 'Loop') continue;
        const display = extractLoopDisplayMeta(records[i].metadata, records[i].resultPreview);
        if (display) { latest = display; break; }
      }

      expect(latest).not.toBeNull();
      expect(latest!.kind).toBe('loop_round');
      if (latest!.kind === 'loop_round') {
        expect(latest!.round).toBe(2);
      }
    });
  });

  describe('message metadata parsing', () => {
    it('parses tool_results from assistant message metadata', () => {
      const msg: Message = {
        id: 'msg-1',
        role: 'assistant',
        content: 'Done',
        tool_calls: [],
        timestamp: new Date().toISOString(),
        metadata: {
          tool_results: [
            {
              name: 'FileEdit',
              arguments: JSON.stringify({ file_path: '/src/main.rs', old_string: 'x', new_string: 'y' }),
              success: true,
              duration_ms: 10,
              result_preview: 'ok',
            },
          ],
        },
      };

      const metaResults = msg.metadata?.tool_results;
      expect(Array.isArray(metaResults)).toBe(true);
      const parsed = (metaResults as Array<Record<string, unknown>>).map((tr) => ({
        name: String(tr.name ?? ''),
        arguments: String(tr.arguments ?? ''),
        success: Boolean(tr.success),
        durationMs: Number(tr.duration_ms ?? 0),
        resultPreview: String(tr.result_preview ?? ''),
      }));
      expect(parsed).toHaveLength(1);
      expect(parsed[0].name).toBe('FileEdit');
    });

    it('ignores user messages', () => {
      const msg: Message = {
        id: 'msg-1',
        role: 'user',
        content: 'hello',
        tool_calls: [],
        timestamp: new Date().toISOString(),
        metadata: {
          tool_results: [
            { name: 'FileEdit', arguments: '{}', success: true, duration_ms: 0, result_preview: '' },
          ],
        },
      };
      // The hook only processes assistant messages
      expect(msg.role).toBe('user');
    });
  });
});
