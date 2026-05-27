import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const sourceRoot = join(process.cwd(), 'src');
const sourceExtensions = new Set(['.css', '.tsx']);

function collectSourceFiles(dir: string): string[] {
  return readdirSync(dir)
    .flatMap((entry) => {
      const path = join(dir, entry);
      const stat = statSync(path);
      if (stat.isDirectory()) return collectSourceFiles(path);
      return sourceExtensions.has(path.slice(path.lastIndexOf('.'))) ? [path] : [];
    });
}

describe('loading animation performance budget', () => {
  it('keeps continuous loading indicators on lightweight shared animations', () => {
    const offenders = collectSourceFiles(sourceRoot)
      .flatMap((path) => {
        const lines = readFileSync(path, 'utf8').split('\n');
        return lines
          .map((line, index) => ({ line, index }))
          .filter(({ line }) => {
            if (/animate-spin|--spinning|spinner|Loader2/.test(line)) return true;
            const continuous = /animation:\s*[^;]*infinite/.test(line);
            const allowed = /animation:\s*busy(?:Breathe|DotBreathe|SkeletonBreathe)\b[^;]*infinite/.test(line);
            return continuous && !allowed;
          })
          .map(({ line, index }) => `${relative(sourceRoot, path)}:${index + 1}: ${line.trim()}`);
      });

    expect(offenders).toEqual([]);
  });
});
