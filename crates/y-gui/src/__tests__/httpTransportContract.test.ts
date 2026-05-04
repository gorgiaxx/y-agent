import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HttpTransport } from '../lib/httpTransport';

type FetchCall = {
  url: string;
  init: RequestInit;
};

const jsonResponse = (value: unknown) =>
  new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });

function installFetchMock(responseValue: unknown = { ok: true }) {
  const calls: FetchCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), init: init ?? {} });
    return jsonResponse(responseValue);
  });
  vi.stubGlobal('fetch', fetchMock);
  return { calls, fetchMock };
}

describe('HttpTransport contract mapping', () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: vi.fn((key: string) => store.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => { store.set(key, value); }),
      removeItem: vi.fn((key: string) => { store.delete(key); }),
      clear: vi.fn(() => { store.clear(); }),
    });
    vi.stubGlobal('EventSource', class {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSED = 2;
      onopen: ((event: Event) => void) | null = null;
      onmessage: ((event: MessageEvent) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      readonly url: string | URL;
      constructor(url: string | URL) {
        this.url = url;
      }
      addEventListener() {}
      removeEventListener() {}
      dispatchEvent() { return true; }
      close() {}
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('maps chat_send frontend args to the y-web snake_case request body', async () => {
    const { calls } = installFetchMock({ session_id: 's1', run_id: 'r1' });
    const transport = new HttpTransport('http://localhost:3000/', 'token-1');

    await transport.invoke('chat_send', {
      message: 'draw a chart',
      sessionId: 's1',
      providerId: 'p1',
      requestMode: 'image_generation',
      knowledgeCollections: ['docs'],
      contextStartIndex: 3,
      thinkingEffort: 'high',
      planMode: 'auto',
      mcpMode: 'manual',
      mcpServers: ['filesystem'],
      imageGenerationOptions: { watermark: true, max_images: 2, size: '1024x1024' },
    });

    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('http://localhost:3000/api/v1/chat/send');
    expect(calls[0].init.method).toBe('POST');
    expect(calls[0].init.headers).toMatchObject({
      Authorization: 'Bearer token-1',
      'Content-Type': 'application/json',
    });
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      message: 'draw a chart',
      session_id: 's1',
      provider_id: 'p1',
      request_mode: 'image_generation',
      knowledge_collections: ['docs'],
      context_start_index: 3,
      thinking_effort: 'high',
      plan_mode: 'auto',
      mcp_mode: 'manual',
      mcp_servers: ['filesystem'],
      image_generation_options: { watermark: true, max_images: 2, size: '1024x1024' },
    });
  });

  it('routes skill_import to the y-web import endpoint instead of no-oping', async () => {
    const { calls } = installFetchMock({ decision: 'accepted' });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('skill_import', {
      path: '/tmp/example.skill.toml',
      sanitize: false,
    });

    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('http://localhost:3000/api/v1/skills/import');
    expect(calls[0].init.method).toBe('POST');
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      path: '/tmp/example.skill.toml',
      sanitize: false,
    });
  });

  it('normalizes y-web system status to the Tauri command shape', async () => {
    installFetchMock({
      status: {
        version: '0.5.5',
        providers_registered: 3,
        tools_registered: 12,
      },
      diagnostics: { trace_store_ok: true },
    });
    const transport = new HttpTransport('http://localhost:3000');

    const status = await transport.invoke('system_status');

    expect(status).toEqual({
      version: '0.5.5',
      healthy: true,
      provider_count: 3,
      session_count: null,
    });
  });

  it('unwraps content payloads for string-returning command parity', async () => {
    installFetchMock({ content: 'raw toml content' });
    const transport = new HttpTransport('http://localhost:3000');

    const content = await transport.invoke<string>('config_get_section', {
      section: 'providers',
    });

    expect(content).toBe('raw toml content');
  });

  it('sends MCP config content as the raw y-web JSON document', async () => {
    const { calls } = installFetchMock({ message: 'saved' });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('mcp_config_save', {
      content: {
        mcpServers: {
          filesystem: { command: 'npx', args: ['server'] },
        },
      },
    });

    expect(calls[0].url).toBe('http://localhost:3000/api/v1/config/mcp');
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      mcpServers: {
        filesystem: { command: 'npx', args: ['server'] },
      },
    });
  });

  it('maps background task lifecycle commands to process-scoped endpoints', async () => {
    const { calls } = installFetchMock({ process_id: 'proc-1' });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('background_task_list', {
      sessionId: 'session-a',
    });
    await transport.invoke('background_task_poll', {
      sessionId: 'session-a',
      processId: 'proc-1',
      yieldTimeMs: 50,
      maxOutputBytes: 4096,
    });
    await transport.invoke('background_task_write', {
      sessionId: 'session-a',
      processId: 'proc-1',
      input: 'rs\n',
    });
    await transport.invoke('background_task_kill', {
      sessionId: 'session-a',
      processId: 'proc-1',
    });

    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/sessions/session-a/background-tasks',
      'http://localhost:3000/api/v1/sessions/session-a/background-tasks/proc-1/poll',
      'http://localhost:3000/api/v1/sessions/session-a/background-tasks/proc-1/write',
      'http://localhost:3000/api/v1/sessions/session-a/background-tasks/proc-1/kill',
    ]);
    expect(JSON.parse(String(calls[1].init.body))).toEqual({
      yield_time_ms: 50,
      max_output_bytes: 4096,
    });
    expect(JSON.parse(String(calls[2].init.body))).toEqual({
      input: 'rs\n',
    });
    expect(JSON.parse(String(calls[3].init.body))).toEqual({});
  });

  it('uses relativePath for skill file URLs and unwraps content responses', async () => {
    const { calls } = installFetchMock({ content: 'skill body' });
    const transport = new HttpTransport('http://localhost:3000');

    const content = await transport.invoke<string>('skill_read_file', {
      name: 'writer',
      relativePath: 'docs/read me.md',
    });

    expect(content).toBe('skill body');
    expect(calls[0].url).toBe('http://localhost:3000/api/v1/skills/writer/files/docs/read%20me.md');
  });

  it('documents heartbeat_pong as a lifecycle-only no-op in web mode', async () => {
    const { calls } = installFetchMock();
    const transport = new HttpTransport('http://localhost:3000');

    await expect(transport.invoke('heartbeat_pong')).resolves.toBeUndefined();

    expect(calls).toHaveLength(0);
  });

  it('maps session and workspace command args to y-web request bodies', async () => {
    const { calls } = installFetchMock({ ok: true });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('session_create', { title: 'Daily', agentId: 'coder' });
    await transport.invoke('session_fork', {
      sessionId: 's1',
      messageIndex: 4,
      title: 'Branch',
    });
    await transport.invoke('workspace_assign_session', {
      workspaceId: 'w1',
      sessionId: 's1',
    });
    await transport.invoke('workspace_unassign_session', { sessionId: 's1' });

    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/sessions',
      'http://localhost:3000/api/v1/sessions/s1/fork',
      'http://localhost:3000/api/v1/workspaces/assign',
      'http://localhost:3000/api/v1/workspaces/unassign',
    ]);
    expect(calls.map((call) => JSON.parse(String(call.init.body)))).toEqual([
      { title: 'Daily', agent_id: 'coder' },
      { message_index: 4, title: 'Branch' },
      { workspace_id: 'w1', session_id: 's1' },
      { session_id: 's1' },
    ]);
  });

  it('unwraps session context reset and custom prompt responses', async () => {
    installFetchMock({ index: 7 });
    const transport = new HttpTransport('http://localhost:3000');

    await expect(transport.invoke('session_get_context_reset', {
      sessionId: 's1',
    })).resolves.toBe(7);

    const { calls } = installFetchMock({ prompt: 'Stay concise.' });
    await expect(transport.invoke('session_get_custom_prompt', {
      sessionId: 's1',
    })).resolves.toBe('Stay concise.');
    expect(calls[0].url).toBe('http://localhost:3000/api/v1/sessions/s1/custom-prompt');
  });

  it('maps agent and provider camelCase args to y-web bodies', async () => {
    const { calls } = installFetchMock({ result: 'provider ok' });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('agent_toml_parse', { tomlContent: 'name = "A"' });
    await transport.invoke('agent_save', { id: 'agent-1', tomlContent: 'name = "B"' });
    const message = await transport.invoke<string>('provider_test', {
      providerType: 'openai-compat',
      model: 'test-model',
      apiKey: 'direct',
      apiKeyEnv: '',
      baseUrl: 'http://llm.test/v1',
      headers: { 'X-LLM-Tenant': 'workspace-a' },
      tags: ['chat'],
      capabilities: ['text_chat'],
      probeMode: 'auto',
    });
    await transport.invoke('provider_list_models', {
      baseUrl: 'http://llm.test/v1',
      apiKey: 'direct',
      apiKeyEnv: '',
      headers: { 'X-LLM-Tenant': 'workspace-a' },
    });

    expect(message).toBe('provider ok');
    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/agents/parse-toml',
      'http://localhost:3000/api/v1/agents/agent-1',
      'http://localhost:3000/api/v1/providers/test',
      'http://localhost:3000/api/v1/providers/list-models',
    ]);
    expect(calls.map((call) => JSON.parse(String(call.init.body)))).toEqual([
      { toml_content: 'name = "A"' },
      { toml_content: 'name = "B"' },
      {
        provider_type: 'openai-compat',
        model: 'test-model',
        api_key: 'direct',
        api_key_env: '',
        base_url: 'http://llm.test/v1',
        headers: { 'X-LLM-Tenant': 'workspace-a' },
        tags: ['chat'],
        capabilities: ['text_chat'],
        probe_mode: 'auto',
      },
      {
        base_url: 'http://llm.test/v1',
        api_key: 'direct',
        api_key_env: '',
        headers: { 'X-LLM-Tenant': 'workspace-a' },
      },
    ]);
  });

  it('maps knowledge entry aliases and ingest options to y-web contracts', async () => {
    const { calls } = installFetchMock({ success: true });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('kb_entry_detail', { entryId: 'entry 1', resolution: 'l1' });
    await transport.invoke('kb_entry_delete', { entryId: 'entry 1' });
    await transport.invoke('kb_entry_update_metadata', {
      entryId: 'entry 1',
      documentType: 'spec',
      interpretedTitle: 'Protocol',
      tags: ['web'],
    });
    await transport.invoke('kb_collection_rename', {
      oldName: 'old collection',
      newName: 'new collection',
    });
    await transport.invoke('kb_ingest', {
      source: '/srv/a.md',
      domain: 'docs',
      collection: 'main',
      useLlmSummary: true,
      extractMetadata: true,
    });

    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/knowledge/entries/entry%201?resolution=l1',
      'http://localhost:3000/api/v1/knowledge/entries/entry%201',
      'http://localhost:3000/api/v1/knowledge/entries/entry%201/metadata',
      'http://localhost:3000/api/v1/knowledge/collections/old%20collection/rename',
      'http://localhost:3000/api/v1/knowledge/ingest',
    ]);
    expect(JSON.parse(String(calls[2].init.body))).toEqual({
      document_type: 'spec',
      interpreted_title: 'Protocol',
      tags: ['web'],
    });
    expect(JSON.parse(String(calls[3].init.body))).toEqual({
      new_name: 'new collection',
    });
    expect(JSON.parse(String(calls[4].init.body))).toEqual({
      source: '/srv/a.md',
      domain: 'docs',
      collection: 'main',
      use_llm_summary: true,
      extract_metadata: true,
    });
  });

  it('unwraps automation requests and preserves Tauri-style return parity', async () => {
    const { calls } = installFetchMock({ message: 'deleted' });
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('schedule_create', {
      request: { name: 'Nightly', workflow_id: 'wf1', trigger: { Cron: '* * * * *' } },
    });
    await transport.invoke('schedule_update', {
      id: 'sch1',
      request: { name: 'Morning' },
    });
    const scheduleDeleted = await transport.invoke<boolean>('schedule_delete', { id: 'sch1' });
    const workflowDeleted = await transport.invoke<boolean>('workflow_delete', { id: 'wf1' });
    await transport.invoke('schedule_execution_history', { scheduleId: 'sch1' });
    await transport.invoke('schedule_trigger_now', { scheduleId: 'sch1' });
    await transport.invoke('workflow_execute', { workflowId: 'wf1' });

    expect(scheduleDeleted).toBe(true);
    expect(workflowDeleted).toBe(true);
    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/schedules',
      'http://localhost:3000/api/v1/schedules/sch1',
      'http://localhost:3000/api/v1/schedules/sch1',
      'http://localhost:3000/api/v1/workflows/wf1',
      'http://localhost:3000/api/v1/schedules/sch1/executions',
      'http://localhost:3000/api/v1/schedules/sch1/trigger',
      'http://localhost:3000/api/v1/workflows/wf1/execute',
    ]);
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      name: 'Nightly',
      workflow_id: 'wf1',
      trigger: { Cron: '* * * * *' },
    });
    expect(JSON.parse(String(calls[1].init.body))).toEqual({ name: 'Morning' });
  });

  it('maps observability, rewind, and memory commands to y-web endpoints', async () => {
    const { calls } = installFetchMock({});
    const transport = new HttpTransport('http://localhost:3000');

    await transport.invoke('observability_history', {
      since: '2026-04-24T00:00:00Z',
      until: '2026-04-24T01:00:00Z',
    });
    await transport.invoke('rewind_execute', {
      sessionId: 's1',
      targetMessageId: 'm1',
    });
    await transport.invoke('rewind_restore_files', {
      sessionId: 's1',
      targetMessageId: 'm2',
    });
    await transport.invoke('memory_stats');

    expect(calls.map((call) => call.url)).toEqual([
      'http://localhost:3000/api/v1/observability/history?since=2026-04-24T00%3A00%3A00Z&until=2026-04-24T01%3A00%3A00Z',
      'http://localhost:3000/api/v1/rewind/s1/execute',
      'http://localhost:3000/api/v1/rewind/s1/restore-files',
      'http://localhost:3000/api/v1/memory-stats',
    ]);
    expect(JSON.parse(String(calls[1].init.body))).toEqual({ target_message_id: 'm1' });
    expect(JSON.parse(String(calls[2].init.body))).toEqual({ target_message_id: 'm2' });
  });

  it('rejects desktop-only user-visible commands in web mode', async () => {
    const { calls } = installFetchMock();
    const transport = new HttpTransport('http://localhost:3000');

    await expect(transport.invoke('skill_open_folder', { name: 'writer' }))
      .rejects
      .toThrow('not supported in the web backend');

    expect(calls).toHaveLength(0);
  });
});
