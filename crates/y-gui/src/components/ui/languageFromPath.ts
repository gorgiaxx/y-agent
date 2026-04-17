const EXT_LANG_MAP: Record<string, string> = {
  // Markup / config
  '.toml': 'toml',
  '.json': 'json',
  '.jsonc': 'json',
  '.json5': 'json',
  '.yaml': 'yaml',
  '.yml': 'yaml',
  '.xml': 'xml',
  '.xsl': 'xml',
  '.xslt': 'xml',
  '.svg': 'xml',
  '.ini': 'ini',
  '.cfg': 'ini',
  '.conf': 'ini',
  '.properties': 'ini',

  // Documentation
  '.md': 'markdown',
  '.markdown': 'markdown',
  '.mdx': 'markdown',
  '.rst': 'restructuredtext',

  // Web
  '.html': 'html',
  '.htm': 'html',
  '.css': 'css',
  '.scss': 'scss',
  '.less': 'less',
  '.js': 'javascript',
  '.jsx': 'javascript',
  '.mjs': 'javascript',
  '.cjs': 'javascript',
  '.ts': 'typescript',
  '.tsx': 'typescript',
  '.mts': 'typescript',
  '.cts': 'typescript',
  '.graphql': 'graphql',
  '.gql': 'graphql',

  // Systems
  '.rs': 'rust',
  '.c': 'c',
  '.h': 'c',
  '.cpp': 'cpp',
  '.cxx': 'cpp',
  '.cc': 'cpp',
  '.hpp': 'cpp',
  '.hxx': 'cpp',
  '.go': 'go',
  '.swift': 'swift',
  '.m': 'objective-c',
  '.mm': 'objective-c',

  // JVM
  '.java': 'java',
  '.kt': 'kotlin',
  '.kts': 'kotlin',
  '.scala': 'scala',
  '.groovy': 'groovy',

  // Scripting
  '.py': 'python',
  '.pyi': 'python',
  '.rb': 'ruby',
  '.php': 'php',
  '.lua': 'lua',
  '.pl': 'perl',
  '.pm': 'perl',
  '.r': 'r',
  '.R': 'r',
  '.dart': 'dart',
  '.ex': 'elixir',
  '.exs': 'elixir',
  '.clj': 'clojure',
  '.cljs': 'clojure',

  // .NET
  '.cs': 'csharp',
  '.fs': 'fsharp',
  '.fsx': 'fsharp',
  '.vb': 'vb',

  // Shell
  '.sh': 'shell',
  '.bash': 'shell',
  '.zsh': 'shell',
  '.fish': 'shell',
  '.ps1': 'powershell',
  '.psm1': 'powershell',
  '.bat': 'bat',
  '.cmd': 'bat',

  // Database
  '.sql': 'sql',
  '.pgsql': 'pgsql',
  '.mysql': 'mysql',

  // Containers / infra
  '.dockerfile': 'dockerfile',

  // Misc
  '.txt': 'plaintext',
  '.log': 'plaintext',
  '.env': 'plaintext',
  '.gitignore': 'plaintext',
};

/** Well-known file names that map to a specific language. */
const FILENAME_LANG_MAP: Record<string, string> = {
  'Dockerfile': 'dockerfile',
  'Makefile': 'makefile',
  'Rakefile': 'ruby',
  'Gemfile': 'ruby',
  'CMakeLists.txt': 'cmake',
  '.gitignore': 'plaintext',
  '.dockerignore': 'plaintext',
  '.editorconfig': 'ini',
};

export function languageFromPath(filePath: string): string {
  const sep = filePath.lastIndexOf('/');
  const fileName = sep === -1 ? filePath : filePath.slice(sep + 1);

  // Check exact filename first (Dockerfile, Makefile, etc.)
  const byName = FILENAME_LANG_MAP[fileName];
  if (byName) return byName;

  // Then check file extension
  const lastDot = fileName.lastIndexOf('.');
  if (lastDot === -1) return 'plaintext';
  const ext = fileName.slice(lastDot).toLowerCase();
  return EXT_LANG_MAP[ext] ?? 'plaintext';
}
