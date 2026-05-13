import { useMemo } from 'react';
import { tokenizeAnsi } from './ansiOutputParser';
import './BackgroundTasksPanel.css';

interface AnsiOutputProps {
  content: string;
  className?: string;
}

export function AnsiOutput({ content, className }: AnsiOutputProps) {
  const tokens = useMemo(() => tokenizeAnsi(content), [content]);
  return (
    <pre className={className ?? 'background-task-console-output'}>
      {tokens.map((token, index) => (
        <span
          key={`${index}-${token.text.length}`}
          className={token.className}
          style={token.style}
        >
          {token.text}
        </span>
      ))}
    </pre>
  );
}
