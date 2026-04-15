interface AgentGlyphProps {
  id: string;
  name: string;
  size?: number;
  className?: string;
}

function isCJK(char: string): boolean {
  const cp = char.codePointAt(0)!;
  return (
    (cp >= 0x4e00 && cp <= 0x9fff)
    || (cp >= 0x3400 && cp <= 0x4dbf)
    || (cp >= 0x20000 && cp <= 0x2a6df)
    || (cp >= 0xf900 && cp <= 0xfaff)
  );
}

function isEmoji(char: string): boolean {
  const cp = char.codePointAt(0)!;
  return (
    (cp >= 0x1f600 && cp <= 0x1f64f)
    || (cp >= 0x1f300 && cp <= 0x1f5ff)
    || (cp >= 0x1f680 && cp <= 0x1f6ff)
    || (cp >= 0x1f900 && cp <= 0x1f9ff)
    || (cp >= 0x1fa00 && cp <= 0x1fa6f)
    || (cp >= 0x1fa70 && cp <= 0x1faff)
    || (cp >= 0x2600 && cp <= 0x26ff)
    || (cp >= 0x2700 && cp <= 0x27bf)
    || (cp >= 0xfe00 && cp <= 0xfe0f)
    || (cp >= 0x1f000 && cp <= 0x1f02f)
    || (cp >= 0x1f0a0 && cp <= 0x1f0ff)
    || cp === 0x200d
    || cp === 0x20e3
  );
}

function deriveAbbreviation(name: string): string {
  const chars = [...name];

  for (let i = 0; i < chars.length; i++) {
    if (isEmoji(chars[i])) {
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

  for (const char of chars) {
    if (isCJK(char)) {
      return char;
    }
  }

  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) {
    return '';
  }
  if (words.length === 1) {
    return words[0].slice(0, 2).toUpperCase();
  }
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
}

export function AgentGlyph({
  id,
  name,
  size = 16,
  className = '',
}: AgentGlyphProps) {
  const display = name.trim()
    ? deriveAbbreviation(name)
    : deriveAbbreviation(id);

  return (
    <span
      className={['agent-glyph-fallback', className].filter(Boolean).join(' ')}
      style={{ fontSize: `${Math.max(10, Math.round(size * 0.8))}px` }}
    >
      {display}
    </span>
  );
}
