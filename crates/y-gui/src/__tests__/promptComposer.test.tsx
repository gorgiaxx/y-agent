import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import {
  PromptComposer,
  type PromptSectionForComposer,
} from '../components/prompts/PromptComposer';
import { buildPromptPreview } from '../components/prompts/promptPreview';

const sections: PromptSectionForComposer[] = [
  {
    id: 'core.identity',
    category: 'identity',
    priority: 100,
    content: 'You are y-agent.',
    condition: 'always',
  },
  {
    id: 'core.datetime',
    category: 'context',
    priority: 150,
    content: 'Current date and time: {{datetime}}',
    condition: 'always',
  },
  {
    id: 'core.tool_protocol',
    category: 'behavioral',
    priority: 450,
    content: 'Tool Usage Protocol',
    condition: 'always',
  },
];

describe('PromptComposer', () => {
  it('builds the final preview from custom prompt plus selected functional sections', () => {
    const preview = buildPromptPreview({
      systemPrompt: 'Custom agent rules.',
      selectedSectionIds: ['core.identity', 'core.tool_protocol'],
      promptSections: sections,
      mode: 'general',
    });

    expect(preview).toContain('Custom agent rules.');
    expect(preview).toContain('Tool Usage Protocol');
    expect(preview).not.toContain('You are y-agent.');
  });

  it('renders a final prompt preview alongside the shared prompt inputs', () => {
    const html = renderToStaticMarkup(
      <PromptComposer
        systemPrompt="Custom session rules."
        selectedSectionIds={['core.tool_protocol']}
        promptSections={sections}
        mode="general"
        onSystemPromptChange={() => {}}
        onSectionToggle={() => {}}
      />,
    );

    expect(html).toContain('System Prompt');
    expect(html).toContain('Prompt Sections');
    expect(html).toContain('Final Prompt Preview');
    expect(html).toContain('Custom session rules.');
    expect(html).toContain('Tool Usage Protocol');
  });
});
