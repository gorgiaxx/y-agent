export const OTHER_LABEL = '__other__';

export type AskUserManualAction = 'next' | 'confirm' | null;

interface ResolveAskUserManualActionArgs {
  isLastStep: boolean;
  isMulti: boolean;
  selections: string[];
}

export function resolveAskUserManualAction({
  isLastStep,
  isMulti,
  selections,
}: ResolveAskUserManualActionArgs): AskUserManualAction {
  const hasCustomTextAnswer = selections.includes(OTHER_LABEL);
  if (!isMulti && !hasCustomTextAnswer) return null;
  return isLastStep ? 'confirm' : 'next';
}
