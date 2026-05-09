export interface PromptSectionForComposer {
  id: string;
  category: string;
  priority: number;
  content: string;
  condition?: string | null;
}

const CUSTOM_PROMPT_REPLACED_SECTION_IDS = new Set([
  'core.identity',
  'core.guidelines',
  'core.security',
  'core.persona',
]);

export function buildPromptPreview({
  systemPrompt,
  selectedSectionIds,
  promptSections,
  mode = 'general',
}: {
  systemPrompt: string;
  selectedSectionIds: string[];
  promptSections: PromptSectionForComposer[];
  mode?: string;
}): string {
  const trimmedPrompt = systemPrompt.trim();
  const selected = new Set(selectedSectionIds);
  const hasExplicitSelection = selected.size > 0;
  const content: string[] = [];

  if (trimmedPrompt) {
    content.push(trimmedPrompt);
  }

  const sortedSections = [...promptSections].sort((left, right) => (
    left.priority - right.priority || left.id.localeCompare(right.id)
  ));

  for (const section of sortedSections) {
    if (hasExplicitSelection && !selected.has(section.id)) {
      continue;
    }
    if (!hasExplicitSelection && trimmedPrompt && CUSTOM_PROMPT_REPLACED_SECTION_IDS.has(section.id)) {
      continue;
    }
    if (!isSectionActiveForMode(section.condition, mode)) {
      continue;
    }
    const sectionContent = resolveTemplatePlaceholders(section.content.trim(), section.id);
    if (sectionContent) {
      content.push(sectionContent);
    }
  }

  return content.join('\n\n');
}

function resolveTemplatePlaceholders(content: string, sectionId: string): string {
  if (!content.includes('{{')) return content;
  return content.replace(/\{\{(\w+)\}\}/g, (_match, name) => `[${sectionId}: ${name} -- resolved at runtime]`);
}

function isSectionActiveForMode(condition: string | null | undefined, mode: string): boolean {
  if (!condition || condition === 'Always') {
    return true;
  }
  if (condition.startsWith('ModeIs(')) {
    return condition.includes(`"${mode}"`);
  }
  if (condition.startsWith('ModeNot(')) {
    return !condition.includes(`"${mode}"`);
  }
  if (condition.includes('plan_mode.active') || condition.includes('mcp.enabled')) {
    return false;
  }
  return true;
}
