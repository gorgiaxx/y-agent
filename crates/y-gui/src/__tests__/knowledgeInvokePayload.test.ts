import { describe, expect, it } from 'vitest';

import {
  buildKnowledgeIngestBatchPayload,
  buildKnowledgeIngestPayload,
} from '../hooks/knowledgeInvokePayload';

describe('knowledgeInvokePayload', () => {
  it('builds camelCase ingest payload keys for Tauri commands', () => {
    const payload = buildKnowledgeIngestPayload({
      source: '/tmp/doc.md',
      domain: 'safety',
      collection: 'default',
      options: {
        useLlmSummary: true,
        extractMetadata: true,
      },
    });

    expect(payload).toMatchObject({
      source: '/tmp/doc.md',
      domain: 'safety',
      collection: 'default',
      useLlmSummary: true,
      extractMetadata: true,
    });
    expect(payload).not.toHaveProperty('use_llm_summary');
    expect(payload).not.toHaveProperty('extract_metadata');
  });

  it('normalizes empty domain to null for batch ingest payloads', () => {
    const payload = buildKnowledgeIngestBatchPayload({
      sources: ['/tmp/a.md', '/tmp/b.md'],
      domain: '',
      collection: 'default',
      options: {
        useLlmSummary: true,
        extractMetadata: false,
      },
    });

    expect(payload).toMatchObject({
      sources: ['/tmp/a.md', '/tmp/b.md'],
      domain: null,
      collection: 'default',
      useLlmSummary: true,
      extractMetadata: false,
    });
    expect(payload).not.toHaveProperty('use_llm_summary');
    expect(payload).not.toHaveProperty('extract_metadata');
  });
});
