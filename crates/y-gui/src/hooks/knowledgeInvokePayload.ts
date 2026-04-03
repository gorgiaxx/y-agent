export interface KnowledgeIngestOptions {
  useLlmSummary?: boolean;
  extractMetadata?: boolean;
}

interface KnowledgeIngestPayloadInput {
  source: string;
  domain: string | undefined;
  collection: string;
  options?: KnowledgeIngestOptions;
}

interface KnowledgeIngestBatchPayloadInput {
  sources: string[];
  domain: string | undefined;
  collection: string;
  options?: KnowledgeIngestOptions;
}

export function buildKnowledgeIngestPayload({
  source,
  domain,
  collection,
  options,
}: KnowledgeIngestPayloadInput) {
  return {
    source,
    domain: domain || null,
    collection,
    useLlmSummary: options?.useLlmSummary ?? false,
    extractMetadata: options?.extractMetadata ?? false,
  };
}

export function buildKnowledgeIngestBatchPayload({
  sources,
  domain,
  collection,
  options,
}: KnowledgeIngestBatchPayloadInput) {
  return {
    sources,
    domain: domain || null,
    collection,
    useLlmSummary: options?.useLlmSummary ?? false,
    extractMetadata: options?.extractMetadata ?? false,
  };
}
