export const OPENAI_RESPONSE_API_LABEL = 'OpenAI Response API';

export const PROVIDER_TYPE_OPTIONS = [
  { value: 'openai', label: OPENAI_RESPONSE_API_LABEL },
  { value: 'openai-compat', label: 'OpenAI-compatible (vLLM, LiteLLM...)' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'gemini', label: 'Gemini' },
  { value: 'deepseek', label: 'DeepSeek' },
  { value: 'ollama', label: 'Ollama' },
  { value: 'azure', label: 'Azure OpenAI' },
] as const;
