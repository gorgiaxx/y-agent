export interface TextSegment {
  text: string;
  isMatch: boolean;
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function splitTextByQuery(text: string, query: string): TextSegment[] {
  if (!query) return [{ text, isMatch: false }];

  const regex = new RegExp(`(${escapeRegex(query)})`, 'gi');
  const parts = text.split(regex);

  if (parts.length === 1) return [{ text, isMatch: false }];

  return parts
    .filter((part) => part !== '')
    .map((part) => ({
      text: part,
      isMatch: regex.test(part) ? (regex.lastIndex = 0, true) : false,
    }));
}
