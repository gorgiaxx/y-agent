/** Parse a JSON-encoded tags string into a string array. Returns [] on failure. */
export function parseTags(tagsStr: string): string[] {
  try {
    const arr = JSON.parse(tagsStr);
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
}
