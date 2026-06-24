import { describe, expect, it } from 'vitest';
import { isSkillCreatedEvent } from '../hooks/useSkills';
import type { DiagnosticsGatewayEvent } from '../types';

describe('isSkillCreatedEvent', () => {
  it('matches a successful skill-creator sub-agent completion', () => {
    const ev: DiagnosticsGatewayEvent = {
      type: 'subagent_completed',
      trace_id: 't1',
      session_id: 's1',
      agent_name: 'skill-creator',
      success: true,
    };
    expect(isSkillCreatedEvent(ev)).toBe(true);
  });

  it('ignores a failed skill-creator completion', () => {
    const ev: DiagnosticsGatewayEvent = {
      type: 'subagent_completed',
      trace_id: 't1',
      session_id: 's1',
      agent_name: 'skill-creator',
      success: false,
    };
    expect(isSkillCreatedEvent(ev)).toBe(false);
  });

  it('ignores completions from other agents', () => {
    const ev: DiagnosticsGatewayEvent = {
      type: 'subagent_completed',
      trace_id: 't1',
      session_id: 's1',
      agent_name: 'code-reviewer',
      success: true,
    };
    expect(isSkillCreatedEvent(ev)).toBe(false);
  });

  it('ignores unrelated diagnostics events', () => {
    const ev = {
      type: 'tool_call_completed',
      agent_name: 'skill-creator',
    } as unknown as DiagnosticsGatewayEvent;
    expect(isSkillCreatedEvent(ev)).toBe(false);
  });

  it('tolerates null/undefined payloads', () => {
    expect(isSkillCreatedEvent(null)).toBe(false);
    expect(isSkillCreatedEvent(undefined)).toBe(false);
  });
});
