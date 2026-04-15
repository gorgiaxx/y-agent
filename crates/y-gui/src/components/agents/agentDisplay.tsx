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

/**
 * Check if a character is a CJK ideograph.
 */
function isCJK(char: string): boolean {
  const cp = char.codePointAt(0)!;
  return (
    (cp >= 0x4e00 && cp <= 0x9fff)   // CJK Unified Ideographs
    || (cp >= 0x3400 && cp <= 0x4dbf) // CJK Extension A
    || (cp >= 0x20000 && cp <= 0x2a6df) // CJK Extension B-I
    || (cp >= 0xf900 && cp <= 0xfaff) // CJK Compatibility Ideographs
  );
}

/**
 * Check if a character is an emoji (wide Unicode ranges covering common emojis).
 */
function isEmoji(char: string): boolean {
  const cp = char.codePointAt(0)!;
  return (
    (cp >= 0x1f600 && cp <= 0x1f64f) // Emoticons
    || (cp >= 0x1f300 && cp <= 0x1f5ff) // Misc Symbols and Pictographs
    || (cp >= 0x1f680 && cp <= 0x1f6ff) // Transport and Map
    || (cp >= 0x1f900 && cp <= 0x1f9ff) // Supplemental Symbols and Pictographs
    || (cp >= 0x1fa00 && cp <= 0x1fa6f) // Chess Symbols
    || (cp >= 0x1fa70 && cp <= 0x1faff) // Symbols and Pictographs Extended-A
    || (cp >= 0x2600 && cp <= 0x26ff)   // Misc Symbols
    || (cp >= 0x2700 && cp <= 0x27bf)   // Dingbats
    || (cp >= 0xfe00 && cp <= 0xfe0f)   // Variation Selectors
    || (cp >= 0x1f000 && cp <= 0x1f02f) // Mahjong Tiles
    || (cp >= 0x1f0a0 && cp <= 0x1f0ff) // Playing Cards
    || (cp >= 0x200d && cp <= 0x200d)   // Zero Width Joiner (for compound emoji)
    || (cp >= 0x20e3 && cp <= 0x20e3)   // Combining Enclosing Keycap
  );
}

/**
 * Derive a display glyph from a name string:
 * - If name contains an emoji, return the first emoji character only
 * - If name contains CJK characters, return the first CJK character only
 * - Otherwise, return up to 2 Latin letters (first letter of first two words, or first 2 chars)
 */
function deriveAbbreviation(name: string): string {
  // Iterate over full Unicode code points (handling surrogate pairs)
  const chars = [...name];

  // Check for emoji first
  for (let i = 0; i < chars.length; i++) {
    if (isEmoji(chars[i])) {
      // Return the emoji and any following variation selector or ZWJ sequence
      let result = chars[i];
      for (let j = i + 1; j < chars.length; j++) {
        const cp = chars[j].codePointAt(0)!;
        if (cp === 0x200d || (cp >= 0xfe00 && cp <= 0xfe0f)) {
          result += chars[j];
        } else {
          break;
        }
      }
      return result;
    }
  }

  // Check for CJK
  for (const char of chars) {
    if (isCJK(char)) {
      return char;
    }
  }

  // Fallback: Latin abbreviation -- first letter of first two words, or first 2 chars
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) {
    return '';
  }
  if (words.length === 1) {
    return words[0].slice(0, 2).toUpperCase();
  }
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
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
  id: string;
  name: string;
  size?: number;
  className?: string;
}

export function AgentGlyph({
  id,
  name,
  size = 16,
  className = '',
}: AgentGlyphProps) {
  // If name is non-empty, derive abbreviation from it; otherwise fall back to id text
  const display = name.trim()
    ? deriveAbbreviation(name)
    : deriveAbbreviation(id);

  return (
    <span className={['agent-glyph-fallback', className].filter(Boolean).join(' ')}>
      {display}
    </span>
  );
}
