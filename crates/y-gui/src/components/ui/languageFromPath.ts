const EXT_LANG_MAP: Record<string, string> = {
  '.toml': 'toml',
  '.md': 'markdown',
  '.markdown': 'markdown',
  '.json': 'json',
  '.py': 'python',
  '.rs': 'rust',
  '.js': 'javascript',
  '.jsx': 'javascript',
  '.ts': 'typescript',
  '.tsx': 'typescript',
  '.yaml': 'yaml',
  '.yml': 'yaml',
  '.css': 'css',
  '.html': 'html',
  '.xml': 'xml',
  '.sh': 'shell',
  '.bash': 'shell',
  '.zsh': 'shell',
  '.txt': 'plaintext',
  '.log': 'plaintext',
  '.env': 'plaintext',
};

export function languageFromPath(filePath: string): string {
  const lastDot = filePath.lastIndexOf('.');
  if (lastDot === -1) return 'plaintext';
  const ext = filePath.slice(lastDot).toLowerCase();
  return EXT_LANG_MAP[ext] ?? 'plaintext';
}
