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
    expect(preview).toContain('You are y-agent.');
  });

  it('excludes replaced sections when no explicit selection is given', () => {
    const preview = buildPromptPreview({
      systemPrompt: 'Custom agent rules.',
      selectedSectionIds: [],
      promptSections: sections,
      mode: 'general',
    });

    expect(preview).toContain('Custom agent rules.');
    expect(preview).toContain('Tool Usage Protocol');
    expect(preview).not.toContain('You are y-agent.');
  });

  it('resolves template placeholders with descriptive labels', () => {
    const preview = buildPromptPreview({
      systemPrompt: '',
      selectedSectionIds: ['core.datetime'],
      promptSections: sections,
      mode: 'general',
    });

    expect(preview).not.toContain('{{datetime}}');
    expect(preview).toContain('[core.datetime: datetime -- resolved at runtime]');
  });

  it('treats empty selectedSectionIds as all sections selected in the UI', () => {
    const html = renderToStaticMarkup(
      <PromptComposer
        systemPrompt=""
        selectedSectionIds={[]}
        promptSections={sections}
        mode="general"
        onSystemPromptChange={() => {}}
        onSelectedSectionIdsChange={() => {}}
      />,
    );

    expect(html).toContain('agent-editor-checkbox-card--active');
  });

  it('renders a final prompt preview alongside the shared prompt inputs', () => {
    const html = renderToStaticMarkup(
      <PromptComposer
        systemPrompt="Custom session rules."
        selectedSectionIds={['core.tool_protocol']}
        promptSections={sections}
        mode="general"
        onSystemPromptChange={() => {}}
        onSelectedSectionIdsChange={() => {}}
      />,
    );

    expect(html).toContain('System Prompt');
    expect(html).toContain('Prompt Sections');
    expect(html).toContain('Final Prompt Preview');
    expect(html).toContain('prompt-composer-left');
    expect(html).toContain('prompt-composer-right');
    expect(html).toContain('settings-group-title');
    expect(html).toContain('prompt-composer-preview-editor');
  });

  it('can render only prompt inputs when an outer layout owns the preview', () => {
    const html = renderToStaticMarkup(
      <PromptComposer
        systemPrompt="Custom session rules."
        selectedSectionIds={['core.tool_protocol']}
        promptSections={sections}
        mode="general"
        showPreview={false}
        onSystemPromptChange={() => {}}
        onSelectedSectionIdsChange={() => {}}
      />,
    );

    expect(html).toContain('System Prompt');
    expect(html).toContain('Prompt Sections');
    expect(html).toContain('prompt-composer--inputs-only');
    expect(html).not.toContain('Final Prompt Preview');
    expect(html).not.toContain('prompt-composer-right');
  });
});
