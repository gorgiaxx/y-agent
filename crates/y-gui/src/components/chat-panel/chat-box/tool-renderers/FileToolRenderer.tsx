import { useState } from 'react';
import { FilePenLine, FilePlus2, FileSearch, ChevronRight, ExternalLink } from 'lucide-react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { formatDuration } from '../../../../utils/formatDuration';
import { tryParseJson, extractFileToolMeta, basename, inferLanguage, truncateForTag } from '../toolCallUtils';
import type { FileToolType } from '../toolCallUtils';
import { useResolvedTheme } from '../../../../hooks/useTheme';
import { platform, logger } from '../../../../lib';
import { CodeBlock } from '../MessageShared';
import { buildFileContextMenuItems } from '../fileContextMenu';
import { useContextMenu } from '../useContextMenu';
import { FileDiffView, DetailSections } from './shared';
import type { ToolRendererProps } from './types';

const FILE_ICONS: Record<FileToolType, typeof FilePenLine> = {
  read: FileSearch,
  edit: FilePenLine,
  write: FilePlus2,
};

const FILE_LABELS: Record<FileToolType, string> = {
  read: 'Read',
  edit: 'Update',
  write: 'Create',
};

export function FileToolRenderer({
  toolCall, status, durationMs, result,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);
  const contextMenu = useContextMenu();

  const fileMeta = extractFileToolMeta(toolCall.name, toolCall.arguments);

  const fallbackType: FileToolType = toolCall.name === 'FileEdit' ? 'edit'
    : toolCall.name === 'FileRead' ? 'read' : 'write';
  const toolType = fileMeta?.toolType ?? fallbackType;
  const fileName = fileMeta ? truncateForTag(basename(fileMeta.filePath)) : toolCall.name;
  const FileIcon = FILE_ICONS[toolType];
  const fileLabel = FILE_LABELS[toolType];
  const hasDiff = fileMeta?.toolType === 'edit'
    && fileMeta.oldString !== undefined
    && fileMeta.newString !== undefined;
  const showDiff = hasDiff && status !== 'error';

  let fileContent: string | null = null;
  if (toolType === 'write') {
    const argsObj = tryParseJson(toolCall.arguments);
    fileContent = argsObj && typeof argsObj.content === 'string' ? argsObj.content : null;
  } else if (toolType === 'read' && result) {
    const resObj = tryParseJson(result);
    fileContent = resObj && typeof resObj.content === 'string' ? resObj.content : null;
  }

  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const language = fileMeta ? inferLanguage(fileMeta.filePath) : 'text';

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = showDiff || !!fileContent || hasExpandable || status === 'error';
  const canOpenInIde = !!fileMeta && toolType !== 'read' && platform.capabilities.openInIde;
  const canOpenFile = !!fileMeta;
  const canRevealInFileManager = !!fileMeta && platform.capabilities.revealFileManager;

  const handleOpenInIde = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    if (!fileMeta) return;

    try {
      await platform.openPathInIde(fileMeta.filePath);
    } catch (error) {
      logger.error('Failed to open file in IDE', error);
    }
  };

  const handleContextMenu = (event: React.MouseEvent) => {
    if (!fileMeta) return;

    const items = buildFileContextMenuItems(fileMeta.filePath, {
      openInIde: canOpenInIde,
      openFile: canOpenFile,
      revealInFileManager: canRevealInFileManager,
      copyPath: true,
    });
    contextMenu.show(event, items);
  };

  return (
    <div className={`tool-call-file-wrapper ${statusClass}`}>
      <div
        className="tool-call-tag"
        data-file-context-menu={fileMeta ? 'true' : undefined}
        onClick={() => canExpand && setExpanded(!expanded)}
        onContextMenu={handleContextMenu}
        title={fileMeta?.filePath ? truncateForTag(fileMeta.filePath) : toolCall.name}
      >
        <span className="tool-call-action-group">
          <FileIcon size={14} className="tool-call-icon-muted" />
          <span className="tool-call-key">{fileLabel}</span>
        </span>
        <span className="tool-call-monospace-value">{fileName}</span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        {canOpenInIde && (
          <button
            type="button"
            className="tool-call-file-open"
            aria-label={`Open ${fileName} in IDE`}
            title={`Open ${fileName} in IDE`}
            onClick={handleOpenInIde}
          >
            <ExternalLink size={12} />
          </button>
        )}
        {canExpand && (
          <span className={`tool-call-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-detail">
          {showDiff && <FileDiffView oldString={fileMeta!.oldString!} newString={fileMeta!.newString!} />}
          {!showDiff && fileContent && status !== 'error' && (
            <CodeBlock language={language} themeStyle={codeThemeStyle}>
              {fileContent}
            </CodeBlock>
          )}
          {!showDiff && !fileContent && (
            <DetailSections
              displayArgs={displayArgs}
              displayResult={activeResult}
              showRaw={showRaw}
              onToggleRaw={() => setShowRaw(!showRaw)}
            />
          )}
          {!showDiff && fileContent && status === 'error' && (
            <DetailSections
              displayResult={activeResult}
              showRaw={showRaw}
              onToggleRaw={() => setShowRaw(!showRaw)}
            />
          )}
        </div>
      )}
      {contextMenu.rendered}
    </div>
  );
}
