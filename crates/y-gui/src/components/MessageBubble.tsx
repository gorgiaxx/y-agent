import type { Message } from '../types';
import { ToolCallCard } from './ToolCallCard';
import './MessageBubble.css';

interface MessageBubbleProps {
  message: Message;
}

export function MessageBubble({ message }: MessageBubbleProps) {
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';

  return (
    <div className={`message-bubble ${message.role}`}>
      <div className="message-avatar">
        {isUser ? '👤' : isSystem ? '⚙️' : '✨'}
      </div>
      <div className="message-body">
        <div className="message-header">
          <span className="message-role">
            {isUser ? 'You' : isSystem ? 'System' : 'Assistant'}
          </span>
          {message.model && (
            <span className="message-model">{message.model}</span>
          )}
        </div>
        <div className="message-content">
          {message.content}
        </div>
        {message.tool_calls.length > 0 && (
          <div className="message-tool-calls">
            {message.tool_calls.map((tc) => (
              <ToolCallCard key={tc.id} toolCall={tc} />
            ))}
          </div>
        )}
        <div className="message-footer">
          <span className="message-time">
            {new Date(message.timestamp).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
            })}
          </span>
          {message.tokens && (
            <span className="message-tokens">
              {message.tokens.input + message.tokens.output} tokens
            </span>
          )}
          {message.cost !== undefined && message.cost > 0 && (
            <span className="message-cost">
              ${message.cost.toFixed(4)}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
