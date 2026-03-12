import { useRef, useEffect } from 'react';
import { Sparkles, AlertTriangle } from 'lucide-react';
import type { Message } from '../types';
import { MessageBubble } from './MessageBubble';
import './ChatPanel.css';

interface ChatPanelProps {
  messages: Message[];
  isStreaming: boolean;
  error: string | null;
}

export function ChatPanel({ messages, isStreaming, error }: ChatPanelProps) {
  const endRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom on new messages.
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, isStreaming]);

  if (messages.length === 0 && !isStreaming) {
    return (
      <div className="chat-panel">
        <div className="chat-empty">
          <div className="chat-empty-icon">
            <Sparkles size={32} />
          </div>
          <h3 className="chat-empty-title">Welcome to y-agent</h3>
          <p className="chat-empty-subtitle">
            Start a conversation by typing a message below.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="chat-panel">
      <div className="chat-messages">
        {messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}

        {isStreaming && (
          <div className="streaming-indicator">
            <div className="typing-dots">
              <span />
              <span />
              <span />
            </div>
            <span className="streaming-text">Thinking...</span>
          </div>
        )}

        {error && (
          <div className="chat-error">
            <span className="error-icon"><AlertTriangle size={14} /></span>
            <span className="error-text">{error}</span>
          </div>
        )}

        <div ref={endRef} />
      </div>
    </div>
  );
}
