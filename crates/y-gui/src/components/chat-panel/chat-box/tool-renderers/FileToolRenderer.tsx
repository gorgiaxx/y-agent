import { useState, useMemo } from 'react';
import { FilePenLine, FilePlus2, FileSearch, ChevronRight } from 'lucide-react';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { formatDuration } from '../../../../utils/formatDuration';
import { tryParseJson, extractFileToolMeta, basename, inferLanguage } from '../toolCallUtils';
import type { FileToolType } from '../toolCallUtils';
import { useResolvedTheme } from '../../../../hooks/useTheme';
import { CodeBlock } from '../MessageShared';
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

  const fileMeta = extractFileToolMeta(toolCall.name, toolCall.arguments);

  const fallbackType: FileToolType = toolCall.name === 'FileEdit' ? 'edit'
    : toolCall.name === 'FileRead' ? 'read' : 'write';
  const toolType = fileMeta?.toolType ?? fallbackType;
  const fileName = fileMeta ? basename(fileMeta.filePath) : toolCall.name;
  const FileIcon = FILE_ICONS[toolType];
  const fileLabel = FILE_LABELS[toolType];
  const hasDiff = fileMeta?.toolType === 'edit'
    && fileMeta.oldString !== undefined
    && fileMeta.newString !== undefined;
  const showDiff = hasDiff && status !== 'error';

  const fileContent = useMemo(() => {
    if (toolType === 'write') {
      const argsObj = tryParseJson(toolCall.arguments);
      if (!argsObj) return null;
      return typeof argsObj.content === 'string' ? argsObj.content : null;
    }
    if (toolType === 'read' && result) {
      const resObj = tryParseJson(result);
      if (!resObj) return null;
      return typeof resObj.content === 'string' ? resObj.content : null;
    }
    return null;
  }, [toolType, toolCall.arguments, result]);

  const resolvedTheme = useResolvedTheme();
  const codeThemeStyle = resolvedTheme === 'light' ? oneLight : oneDark;
  const language = fileMeta ? inferLanguage(fileMeta.filePath) : 'text';

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;
  const canExpand = showDiff || !!fileContent || hasExpandable || status === 'error';

  return (
    <div className={`tool-call-file-wrapper ${statusClass}`}>
      <div
        className="tool-call-file-tag"
        onClick={() => canExpand && setExpanded(!expanded)}
        title={fileMeta?.filePath ?? toolCall.name}
      >
        <span className="tool-call-file-action-group">
          <FileIcon size={14} className="tool-call-file-icon" />
          <span className="tool-call-file-action">{fileLabel}</span>
        </span>
        <span className="tool-call-file-name">{fileName}</span>
        <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
        {durationMs !== undefined && (
          <span className="tool-call-duration">{formatDuration(durationMs)}</span>
        )}
        {canExpand && (
          <span className={`tool-call-file-chevron ${expanded ? 'expanded' : ''}`}>
            <ChevronRight size={12} />
          </span>
        )}
      </div>
      {expanded && (
        <div className="tool-call-file-detail">
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
    </div>
  );
}
