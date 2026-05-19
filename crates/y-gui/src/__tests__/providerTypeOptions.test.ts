import { describe, expect, it } from 'vitest';

import {
  OPENAI_RESPONSE_API_LABEL,
  PROVIDER_TYPE_OPTIONS,
} from '../components/settings/providerTypeOptions';

describe('provider type labels', () => {
  it('names the official OpenAI provider as OpenAI Response API', () => {
    expect(OPENAI_RESPONSE_API_LABEL).toBe('OpenAI Response API');
    expect(PROVIDER_TYPE_OPTIONS.find((option) => option.value === 'openai')?.label)
      .toBe('OpenAI Response API');
  });
});
