const IMAGE_EXTENSIONS = /\.(png|jpe?g|gif|webp|bmp|svg|ico|tiff?)$/i;

export function isAbsoluteFilePath(href: unknown): href is string {
  if (typeof href !== 'string') return false;
  return href.startsWith('/') || /^[A-Z]:[/\\]/i.test(href);
}

export function isFileUrl(href: unknown): href is string {
  return typeof href === 'string' && href.startsWith('file://');
}

export function fileUrlToPath(href: string): string {
  return href.replace(/^file:\/\//, '');
}

export function isLocalPath(href: unknown): href is string {
  return isAbsoluteFilePath(href) || isFileUrl(href);
}

export function resolveLocalPath(href: string): string {
  if (isFileUrl(href)) return fileUrlToPath(href);
  return href;
}

export function isImagePath(href: string): boolean {
  return IMAGE_EXTENSIONS.test(href);
}

export function isWebUrl(href: unknown): href is string {
  return typeof href === 'string' && /^https?:\/\//i.test(href);
}
