import type { Components } from 'react-markdown';
import type { ContentSegment } from '../../../hooks/useStreamContent';
import type { ToolResultRecord } from '../../../hooks/chatStreamTypes';
import type { InterleavedSegment } from '../../../hooks/useInterleavedSegments';
import { ToolCallCard } from './ToolCallCard';
import { ThinkingCard } from './ThinkingCard';
import { ThinkContentBlock } from './ThinkContentBlock';
import { extractThinkTags } from './messageUtils';

type ToolStatus = 'running' | 'success' | 'error';

function toolRecordStatus(record: ToolResultRecord): ToolStatus {
  if (record.state === 'running') return 'running';
  return record.success ? 'success' : 'error';
}

interface XmlSegmentListProps {
  segments: ContentSegment[];
  toolResultsMap: Map<number, ToolResultRecord>;
  markdownComponents: Components;
  isStreaming?: boolean;
}

export function XmlSegmentList({ segments, toolResultsMap, markdownComponents, isStreaming }: XmlSegmentListProps) {
  return (
    <>
      {segments.map((seg, idx) => {
        if (seg.type === 'text') {
          const think = isStreaming ? extractThinkTags(seg.text) : null;
          return (
            <ThinkContentBlock
              key={`text-${idx}`}
              content={seg.text}
              markdownComponents={markdownComponents}
              isStreaming={think?.isThinkingIncomplete}
              className="markdown-body"
            />
          );
        }
        if (seg.type === 'tool_call') {
          const result = toolResultsMap.get(idx);
          const status: ToolStatus = result ? toolRecordStatus(result) : 'success';
          return (
            <ToolCallCard
              key={`tc-${idx}`}
              toolCall={{
                id: `tc-${idx}`,
                name: seg.toolCall.name,
                arguments: seg.toolCall.arguments,
              }}
              status={status}
              result={result?.resultPreview}
              durationMs={result?.durationMs}
              urlMeta={result?.urlMeta}
              metadata={result?.metadata}
            />
          );
        }
        return null;
      })}
    </>
  );
}

interface NativeSegmentListProps {
  segments: InterleavedSegment[];
  markdownComponents: Components;
}

export function NativeSegmentList({ segments, markdownComponents }: NativeSegmentListProps) {
  return (
    <>
      {segments.map((seg, idx) => {
        if (seg.type === 'reasoning') {
          return (
            <ThinkingCard
              key={`reason-${idx}`}
              content={seg.content}
              isStreaming={seg.isStreaming ?? false}
              durationMs={seg.durationMs}
            />
          );
        }
        if (seg.type === 'text') {
          return (
            <ThinkContentBlock
              key={`text-${idx}`}
              content={seg.text}
              markdownComponents={markdownComponents}
              className="markdown-body"
            />
          );
        }
        if (seg.type === 'tool_result') {
          return (
            <ToolCallCard
              key={`native-tc-${idx}`}
              toolCall={{
                id: `native-${idx}`,
                name: seg.record.name,
                arguments: seg.record.arguments ?? '',
              }}
              status={toolRecordStatus(seg.record)}
              result={seg.record.resultPreview}
              durationMs={seg.record.durationMs}
              urlMeta={seg.record.urlMeta}
              metadata={seg.record.metadata}
            />
          );
        }
        return null;
      })}
    </>
  );
}
