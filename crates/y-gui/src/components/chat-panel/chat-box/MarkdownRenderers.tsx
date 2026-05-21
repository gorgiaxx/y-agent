import React, { useState } from 'react';
import {
  ExternalLink, FileText, FolderOpen, Copy,
  Image as ImageIcon, Download,
} from 'lucide-react';
import { platform } from '../../../lib/platform';
import { logger } from '../../../lib';
import { useContextMenu, type ContextMenuItem } from './useContextMenu';
import { isLocalPath, resolveLocalPath, isImagePath, isWebUrl } from './linkUtils';

interface MarkdownLinkProps {
  href?: string;
  children?: React.ReactNode;
}

function basename(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/');
  return parts[parts.length - 1] || path;
}

export function MarkdownLink({ href, children }: MarkdownLinkProps) {
  const { show, rendered } = useContextMenu();

  if (!href) {
    return <span>{children}</span>;
  }

  if (isWebUrl(href)) {
    return (
      <>
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="md-link md-link-web"
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            platform.openUrl(href).catch((err) =>
              logger.error('[MarkdownLink] failed to open URL:', href, err),
            );
          }}
          onContextMenu={(e) => {
            const items: ContextMenuItem[] = [
              {
                label: 'Open in Browser',
                icon: <ExternalLink size={14} />,
                onClick: () => {
                  platform.openUrl(href).catch((err) =>
                    logger.error('[MarkdownLink] failed to open URL:', href, err),
                  );
                },
              },
              {
                label: 'Copy URL',
                icon: <Copy size={14} />,
                onClick: () => { navigator.clipboard.writeText(href); },
              },
            ];
            show(e, items);
          }}
        >
          {children}
        </a>
        {rendered}
      </>
    );
  }

  if (isLocalPath(href)) {
    return <FileLink path={resolveLocalPath(href)}>{children}</FileLink>;
  }

  return <span className="md-link-plain">{children}</span>;
}

interface FileLinkProps {
  path: string;
  children?: React.ReactNode;
}

function FileLink({ path, children }: FileLinkProps) {
  const { show, rendered } = useContextMenu();
  const isImage = isImagePath(path);
  const canOpenIde = platform.capabilities.openInIde;
  const canReveal = platform.capabilities.revealFileManager;
  const fileName = basename(path);
  const childText = typeof children === 'string' ? children : null;
  const displayName = childText && childText !== path ? childText : fileName;

  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (isImage) {
      platform.openUrl(`file://${path}`).catch((err) =>
        logger.error('[FileLink] failed to open image:', path, err),
      );
    } else if (canOpenIde) {
      platform.openPathInIde(path).catch((err) =>
        logger.error('[FileLink] failed to open in IDE:', path, err),
      );
    } else if (canReveal) {
      platform.revealInFileManager(path).catch((err) =>
        logger.error('[FileLink] failed to reveal:', path, err),
      );
    }
  };

  const handleOpenIde = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    platform.openPathInIde(path).catch((err) =>
      logger.error('[FileLink] failed to open in IDE:', path, err),
    );
  };

  const handleReveal = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    platform.revealInFileManager(path).catch((err) =>
      logger.error('[FileLink] failed to reveal:', path, err),
    );
  };

  const handleCopy = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    navigator.clipboard.writeText(path);
  };

  const handleContextMenu = (e: React.MouseEvent) => {
    const items: ContextMenuItem[] = [];
    if (canOpenIde && !isImage) {
      items.push({
        label: 'Open in IDE',
        icon: <ExternalLink size={14} />,
        onClick: () => {
          platform.openPathInIde(path).catch((err) =>
            logger.error('[FileLink] failed to open in IDE:', path, err),
          );
        },
      });
    }
    if (isImage) {
      items.push({
        label: 'Open Image',
        icon: <ImageIcon size={14} />,
        onClick: () => {
          platform.openUrl(`file://${path}`).catch((err) =>
            logger.error('[FileLink] failed to open image:', path, err),
          );
        },
      });
    }
    if (canReveal) {
      items.push({
        label: 'Reveal in File Manager',
        icon: <FolderOpen size={14} />,
        onClick: () => {
          platform.revealInFileManager(path).catch((err) =>
            logger.error('[FileLink] failed to reveal:', path, err),
          );
        },
      });
    }
    items.push({
      label: 'Copy Path',
      icon: <Copy size={14} />,
      onClick: () => { navigator.clipboard.writeText(path); },
    });
    show(e, items);
  };

  return (
    <>
      <span
        className="md-file-tag"
        title={path}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
      >
        <span className="md-file-tag-icon">
          {isImage ? <ImageIcon size={14} /> : <FileText size={14} />}
        </span>
        <span className="md-file-tag-name">{displayName}</span>
        <span className="md-file-tag-path">{path}</span>
        <span className="md-file-tag-actions">
          {canOpenIde && !isImage && (
            <button
              type="button"
              className="md-file-tag-btn"
              title="Open in IDE"
              onClick={handleOpenIde}
            >
              <ExternalLink size={12} />
            </button>
          )}
          {canReveal && (
            <button
              type="button"
              className="md-file-tag-btn"
              title="Reveal in File Manager"
              onClick={handleReveal}
            >
              <FolderOpen size={12} />
            </button>
          )}
          <button
            type="button"
            className="md-file-tag-btn"
            title="Copy Path"
            onClick={handleCopy}
          >
            <Copy size={12} />
          </button>
        </span>
      </span>
      {rendered}
    </>
  );
}

interface MarkdownImageProps {
  src?: string;
  alt?: string;
}

export function MarkdownImage({ src, alt }: MarkdownImageProps) {
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState(false);
  const { show, rendered } = useContextMenu();

  if (!src) return null;

  const isLocal = isLocalPath(src);
  const localPath = isLocal ? resolveLocalPath(src) : src;
  const displaySrc = isLocal && platform.isTauri()
    ? platform.convertFileSrc(localPath)
    : src;

  if (error) {
    return (
      <span className="md-image-error" title={localPath}>
        <ImageIcon size={16} />
        <span>{alt || basename(localPath)}</span>
      </span>
    );
  }

  const handleContextMenu = (e: React.MouseEvent) => {
    const items: ContextMenuItem[] = [];
    if (isLocal) {
      items.push({
        label: 'Open Image',
        icon: <ExternalLink size={14} />,
        onClick: () => {
          platform.openUrl(`file://${localPath}`).catch((err) =>
            logger.error('[MarkdownImage] failed to open:', localPath, err),
          );
        },
      });
      if (platform.capabilities.revealFileManager) {
        items.push({
          label: 'Reveal in File Manager',
          icon: <FolderOpen size={14} />,
          onClick: () => {
            platform.revealInFileManager(localPath).catch((err) =>
              logger.error('[MarkdownImage] failed to reveal:', localPath, err),
            );
          },
        });
      }
      items.push({
        label: 'Copy Path',
        icon: <Copy size={14} />,
        onClick: () => { navigator.clipboard.writeText(localPath); },
      });
    } else {
      items.push({
        label: 'Open in Browser',
        icon: <ExternalLink size={14} />,
        onClick: () => {
          platform.openUrl(src).catch((err) =>
            logger.error('[MarkdownImage] failed to open:', src, err),
          );
        },
      });
      items.push({
        label: 'Save Image',
        icon: <Download size={14} />,
        onClick: () => {
          platform.saveRemoteImage(src).catch((err) =>
            logger.error('[MarkdownImage] failed to save:', src, err),
          );
        },
      });
      items.push({
        label: 'Copy URL',
        icon: <Copy size={14} />,
        onClick: () => { navigator.clipboard.writeText(src); },
      });
    }
    show(e, items);
  };

  return (
    <>
      <span className="md-image-wrapper" onContextMenu={handleContextMenu}>
        {!loaded && (
          <span className="md-image-skeleton">
            <span className="md-image-skeleton-shimmer" />
          </span>
        )}
        <img
          src={displaySrc}
          alt={alt || ''}
          className={`md-image ${loaded ? 'md-image-loaded' : 'md-image-loading'}`}
          onLoad={() => setLoaded(true)}
          onError={() => setError(true)}
          loading="lazy"
        />
        {isLocal && loaded && (
          <span className="md-image-path" title={localPath}>{localPath}</span>
        )}
      </span>
      {rendered}
    </>
  );
}
