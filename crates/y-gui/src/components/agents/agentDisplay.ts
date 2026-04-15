type AgentBadgeVariant = 'default' | 'accent' | 'success' | 'error' | 'outline';

export const AGENT_TIER_ORDER = ['UserDefined', 'BuiltIn', 'Dynamic'] as const;

const AGENT_TIER_LABELS: Record<string, string> = {
  BuiltIn: 'Built-in',
  UserDefined: 'User-defined',
  Dynamic: 'Dynamic',
};

const AGENT_TIER_HINTS: Record<string, string> = {
  BuiltIn: 'Bundled defaults',
  UserDefined: 'Local presets',
  Dynamic: 'Runtime-generated',
};

export function formatAgentTierLabel(tier: string): string {
  return AGENT_TIER_LABELS[tier] ?? tier;
}

export function formatAgentTierHint(tier: string): string {
  return AGENT_TIER_HINTS[tier] ?? 'Preset group';
}

export function formatAgentModeLabel(mode: string): string {
  switch (mode) {
    case 'build':
      return 'Build';
    case 'plan':
      return 'Plan';
    case 'explore':
      return 'Explore';
    default:
      return 'General';
  }
}

export function getAgentModeBadgeVariant(mode: string): AgentBadgeVariant {
  switch (mode) {
    case 'build':
      return 'success';
    case 'plan':
      return 'accent';
    case 'explore':
      return 'outline';
    default:
      return 'default';
  }
}
