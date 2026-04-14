import {
  BookOpen,
  Bot,
  Rocket,
  Search,
  Shield,
  Wrench,
  type LucideIcon,
} from 'lucide-react';

type AgentBadgeVariant = 'default' | 'accent' | 'success' | 'error' | 'outline';

const AGENT_ICON_MAP: Record<string, LucideIcon> = {
  bot: Bot,
  build: Wrench,
  builder: Wrench,
  search: Search,
  knowledge: BookOpen,
  book: BookOpen,
  launch: Rocket,
  rocket: Rocket,
  shield: Shield,
};

export const AGENT_ICON_OPTIONS = [
  { label: 'Bot', value: 'bot', Icon: Bot },
  { label: 'Build', value: 'build', Icon: Wrench },
  { label: 'Search', value: 'search', Icon: Search },
  { label: 'Knowledge', value: 'knowledge', Icon: BookOpen },
  { label: 'Launch', value: 'launch', Icon: Rocket },
  { label: 'Shield', value: 'shield', Icon: Shield },
] as const;

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

function fallbackInitials(value?: string | null): string {
  const normalized = value
    ?.replace(/[^a-zA-Z0-9\s]/g, ' ')
    .trim()
    .split(/\s+/)
    .filter(Boolean) ?? [];

  if (normalized.length === 0) {
    return 'AG';
  }

  if (normalized.length === 1) {
    return normalized[0].slice(0, 2).toUpperCase();
  }

  return `${normalized[0][0]}${normalized[1][0]}`.toUpperCase();
}

function normalizeIconToken(icon?: string | null): string {
  return icon?.trim().toLowerCase() ?? '';
}

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

interface AgentGlyphProps {
  icon?: string | null;
  name: string;
  size?: number;
  className?: string;
}

export function AgentGlyph({
  icon,
  name,
  size = 16,
  className = '',
}: AgentGlyphProps) {
  const normalized = normalizeIconToken(icon);
  const Icon = AGENT_ICON_MAP[normalized];

  if (Icon) {
    return <Icon size={size} className={className} />;
  }

  return (
    <span className={['agent-glyph-fallback', className].filter(Boolean).join(' ')}>
      {fallbackInitials(normalized || name)}
    </span>
  );
}
