// Diagnostics panel -- real-time observability into LLM turn lifecycle.

import { useRef, useEffect, useState, useMemo } from 'react';
import { Activity, X, Cpu, Wrench, AlertTriangle, ChevronDown, ChevronRight, Trash2, Maximize2, Minimize2, User, Copy, Check, Filter } from 'lucide-react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism';

import type { DiagnosticsEntry, LlmResponseEvent, ToolResultEvent, LoopLimitEvent, UserMessageEvent } from '../types';
import { computeSummary } from '../hooks/useDiagnostics';
import type { DiagnosticsSummary } from '../hooks/useDiagnostics';
import { useResolvedTheme } from '../hooks/useTheme';
import './DiagnosticsPanel.css';

// Strip hardcoded background from every token rule so the highlighter
// inherits the container's background (set via customStyle) instead of
// painting the theme's own dark color over individual token spans.
const oneDarkNoBackground = Object.fromEntries(
  Object.entries(oneDark).map(([k, v]) => [
    k,
    {
      ...(v as object),
      background: undefined,
      backgroundColor: undefined,
      overflowX: undefined,
    },
  ])
) as typeof oneDark;

const oneLightNoBackground = Object.fromEntries(
  Object.entries(oneLight).map(([k, v]) => [
    k,
    {
      ...(v as object),
      background: undefined,
      backgroundColor: undefined,
      overflowX: undefined,
    },
  ])
) as typeof oneLight;

// -- Time range filter --

type TimeRange = '15m' | '30m' | '1h' | '6h' | '24h' | 'all';

const TIME_RANGE_OPTIONS: { value: TimeRange; label: string }[] = [
  { value: '15m', label: '15 min' },
  { value: '30m', label: '30 min' },
  { value: '1h', label: '1 hour' },
  { value: '6h', label: '6 hours' },
  { value: '24h', label: '24 hours' },
  { value: 'all', label: 'All time' },
];

function getTimeRangeMs(range: TimeRange): number | null {
  switch (range) {
    case '15m': return 15 * 60 * 1000;
    case '30m': return 30 * 60 * 1000;
    case '1h': return 60 * 60 * 1000;
    case '6h': return 6 * 60 * 60 * 1000;
    case '24h': return 24 * 60 * 60 * 1000;
    case 'all': return null;
  }
}

function filterByTimeRange(entries: DiagnosticsEntry[], range: TimeRange): DiagnosticsEntry[] {
  const ms = getTimeRangeMs(range);
  if (ms === null) return entries;
  const cutoff = Date.now() - ms;
  return entries.filter(e => new Date(e.timestamp).getTime() >= cutoff);
}

interface DiagnosticsPanelProps {
  entries: DiagnosticsEntry[];
  summary: DiagnosticsSummary;
  isActive: boolean;
  isGlobal: boolean;
  expanded: boolean;
  onToggleExpand: () => void;
  onClear: () => void;
  onClose: () => void;
}

function UserMessageEntry({ event, timestamp }: { event: UserMessageEvent; timestamp: string }) {
  const [expanded, setExpanded] = useState(false);
  const time = new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const preview = event.content.length > 80 ? event.content.slice(0, 80) + '...' : event.content;

  return (
    <div className="diag-entry diag-user-msg">
      <div className="diag-entry-header" onClick={() => setExpanded(!expanded)}>
        <div className="diag-entry-icon">
          <User size={14} />
        </div>
        <div className="diag-entry-main">
          <span className="diag-entry-title">User Message</span>
          {!expanded && <span className="diag-user-preview">{preview}</span>}
        </div>
        <div className="diag-entry-badges">
          <span className="diag-badge diag-badge-time">{time}</span>
        </div>
        <span className="diag-expand-icon">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </div>
      {expanded && (
        <div className="diag-entry-detail">
          <pre className="diag-user-content">{event.content}</pre>
        </div>
      )}
    </div>
  );
}

function CopyButton({ getText }: { getText: () => string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard.writeText(getText()).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  };
  return (
    <button className="diag-copy-btn" onClick={handleCopy} title="Copy to clipboard">
      {copied ? <Check size={12} /> : <Copy size={12} />}
    </button>
  );
}

function WrapToggle({ wrapped, onToggle }: { wrapped: boolean; onToggle: () => void }) {
  return (
    <button
      className={`diag-wrap-toggle-btn${wrapped ? ' active' : ''}`}
      onClick={onToggle}
      title={wrapped ? 'Disable word wrap' : 'Enable word wrap'}
    >
      {wrapped ? 'Wrap: on' : 'Wrap: off'}
    </button>
  );
}


function LlmEntry({ event, timestamp }: { event: LlmResponseEvent; timestamp: string }) {
  const [expanded, setExpanded] = useState(false);
  const [promptBeautified, setPromptBeautified] = useState(true);
  const [responseBeautified, setResponseBeautified] = useState(true);
  const [promptWrapped, setPromptWrapped] = useState(true);
  const [responseWrapped, setResponseWrapped] = useState(true);
  const hasToolCalls = event.tool_calls_requested.length > 0;
  const time = new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const resolvedTheme = useResolvedTheme();
  const highlighterStyle = resolvedTheme === 'light' ? oneLightNoBackground : oneDarkNoBackground;

  const tryBeautify = (raw: string): string | null => {
    try {
      return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
      return null;
    }
  };

  const promptJsonOk = event.prompt_preview ? tryBeautify(event.prompt_preview) !== null : false;
  const responseJsonOk = event.response_text ? tryBeautify(event.response_text) !== null : false;

  const displayPrompt = (() => {
    if (!event.prompt_preview) return null;
    if (promptBeautified && promptJsonOk) return tryBeautify(event.prompt_preview);
    return event.prompt_preview;
  })();

  const displayResponse = (() => {
    if (!event.response_text) return null;
    if (responseBeautified && responseJsonOk) return tryBeautify(event.response_text);
    return event.response_text;
  })();

  return (
    <div className="diag-entry diag-llm">
      <div className="diag-entry-header" onClick={() => setExpanded(!expanded)}>
        <div className="diag-entry-icon">
          <Cpu size={14} />
        </div>
        <div className="diag-entry-main">
          <span className="diag-entry-title">
            LLM Call #{event.iteration}
          </span>
          <span className="diag-entry-model">{event.model}</span>
        </div>
        <div className="diag-entry-badges">
          <span className="diag-badge diag-badge-tokens">
            {(event.input_tokens + event.output_tokens).toLocaleString()} tok
          </span>
          <span className="diag-badge diag-badge-time">{event.duration_ms}ms</span>
          {hasToolCalls && (
            <span className="diag-badge diag-badge-tools">
              {event.tool_calls_requested.length} tool{event.tool_calls_requested.length > 1 ? 's' : ''}
            </span>
          )}
        </div>
        <span className="diag-expand-icon">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </div>
      {expanded && (
        <div className="diag-entry-detail">
          <div className="diag-detail-grid">
            <div className="diag-detail-item">
              <span className="diag-detail-label">Input tokens</span>
              <span className="diag-detail-value">{event.input_tokens.toLocaleString()}</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Output tokens</span>
              <span className="diag-detail-value">{event.output_tokens.toLocaleString()}</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Cost</span>
              <span className="diag-detail-value">${event.cost_usd.toFixed(4)}</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Duration</span>
              <span className="diag-detail-value">{event.duration_ms}ms</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Time</span>
              <span className="diag-detail-value">{time}</span>
            </div>
          </div>
          {hasToolCalls && (
            <div className="diag-detail-tools">
              <span className="diag-detail-label">Requested tools</span>
              <div className="diag-tool-tags">
                {event.tool_calls_requested.map((name, i) => (
                  <span key={i} className="diag-tool-tag">{name}</span>
                ))}
              </div>
            </div>
          )}
          {displayPrompt !== null && (
            <div className="diag-result-preview">
              <div className="diag-result-header">
                <span className="diag-detail-label">Input (sent)</span>
                <CopyButton getText={() => displayPrompt ?? ''} />
                {promptBeautified && promptJsonOk && (
                  <WrapToggle wrapped={promptWrapped} onToggle={() => setPromptWrapped((v) => !v)} />
                )}
                <button
                  className={`diag-beautify-btn ${!promptJsonOk ? 'disabled' : ''}`}
                  onClick={() => setPromptBeautified(!promptBeautified)}
                  disabled={!promptJsonOk}
                  title={promptJsonOk ? (promptBeautified ? 'Show raw' : 'Beautify JSON') : 'Not valid JSON'}
                >
                  {promptBeautified ? 'Raw' : 'Beautify'}
                </button>
              </div>
              {promptBeautified && promptJsonOk ? (
                <div className="diag-highlighted-block">
                  <SyntaxHighlighter
                    style={highlighterStyle}
                    language="json"
                    PreTag="div"
                    wrapLongLines={promptWrapped}
                    codeTagProps={{ style: { whiteSpace: promptWrapped ? 'pre-wrap' : 'pre' } }}
                    customStyle={{
                      margin: 0,
                      padding: '8px',
                      fontSize: '11px',
                      lineHeight: '1.5',
                      overflow: 'auto',
                      maxHeight: '320px',
                    }}
                  >
                    {displayPrompt ?? ''}
                  </SyntaxHighlighter>
                </div>
              ) : (
                <pre className="diag-result-code">{displayPrompt}</pre>
              )}
            </div>
          )}
          {displayResponse !== null && (
            <div className="diag-result-preview">
              <div className="diag-result-header">
                <span className="diag-detail-label">Output (received)</span>
                <CopyButton getText={() => displayResponse ?? ''} />
                {responseBeautified && responseJsonOk && (
                  <WrapToggle wrapped={responseWrapped} onToggle={() => setResponseWrapped((v) => !v)} />
                )}
                <button
                  className={`diag-beautify-btn ${!responseJsonOk ? 'disabled' : ''}`}
                  onClick={() => setResponseBeautified(!responseBeautified)}
                  disabled={!responseJsonOk}
                  title={responseJsonOk ? (responseBeautified ? 'Show raw' : 'Beautify JSON') : 'Not valid JSON'}
                >
                  {responseBeautified ? 'Raw' : 'Beautify'}
                </button>
              </div>
              {responseBeautified && responseJsonOk ? (
                <div className="diag-highlighted-block">
                  <SyntaxHighlighter
                    style={highlighterStyle}
                    language="json"
                    PreTag="div"
                    wrapLongLines={responseWrapped}
                    codeTagProps={{ style: { whiteSpace: responseWrapped ? 'pre-wrap' : 'pre' } }}
                    customStyle={{
                      margin: 0,
                      padding: '8px',
                      fontSize: '11px',
                      lineHeight: '1.5',
                      overflow: 'auto',
                      maxHeight: '320px',
                    }}
                  >
                    {displayResponse ?? ''}
                  </SyntaxHighlighter>
                </div>
              ) : (
                <pre className="diag-result-code">{displayResponse}</pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolEntry({ event, timestamp }: { event: ToolResultEvent; timestamp: string }) {
  const [expanded, setExpanded] = useState(false);
  const [beautified, setBeautified] = useState(true);
  const [resultWrapped, setResultWrapped] = useState(true);
  const time = new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const resolvedTheme = useResolvedTheme();
  const highlighterStyle = resolvedTheme === 'light' ? oneLightNoBackground : oneDarkNoBackground;

  // Attempt to format as pretty JSON; return null if not valid JSON.
  const tryBeautify = (raw: string): string | null => {
    try {
      return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
      return null;
    }
  };

  const displayResult = (() => {
    if (!event.result_preview) return null;
    if (!beautified) return event.result_preview;
    return tryBeautify(event.result_preview) ?? event.result_preview;
  })();

  const canBeautify = event.result_preview ? tryBeautify(event.result_preview) !== null : false;
  const isJsonBeautified = beautified && canBeautify;

  return (
    <div className={`diag-entry diag-tool ${event.success ? 'diag-tool-ok' : 'diag-tool-fail'}`}>
      <div className="diag-entry-header" onClick={() => setExpanded(!expanded)}>
        <div className="diag-entry-icon">
          <Wrench size={14} />
        </div>
        <div className="diag-entry-main">
          <span className="diag-entry-title">{event.name}</span>
          <span className={`diag-status-dot ${event.success ? 'success' : 'error'}`} />
        </div>
        <div className="diag-entry-badges">
          <span className={`diag-badge ${event.success ? 'diag-badge-ok' : 'diag-badge-err'}`}>
            {event.success ? 'OK' : 'FAIL'}
          </span>
          <span className="diag-badge diag-badge-time">{event.duration_ms}ms</span>
        </div>
        <span className="diag-expand-icon">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </div>
      {expanded && (
        <div className="diag-entry-detail">
          <div className="diag-detail-grid">
            <div className="diag-detail-item">
              <span className="diag-detail-label">Status</span>
              <span className="diag-detail-value">{event.success ? 'Success' : 'Failed'}</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Duration</span>
              <span className="diag-detail-value">{event.duration_ms}ms</span>
            </div>
            <div className="diag-detail-item">
              <span className="diag-detail-label">Time</span>
              <span className="diag-detail-value">{time}</span>
            </div>
          </div>
          {event.result_preview && (
            <div className="diag-result-preview">
              <div className="diag-result-header">
                <span className="diag-detail-label">Result</span>
                <CopyButton getText={() => displayResult ?? ''} />
                {isJsonBeautified && (
                  <WrapToggle wrapped={resultWrapped} onToggle={() => setResultWrapped((v) => !v)} />
                )}
                <button
                  className={`diag-beautify-btn ${!canBeautify ? 'disabled' : ''}`}
                  onClick={() => setBeautified(!beautified)}
                  disabled={!canBeautify}
                  title={canBeautify ? (beautified ? 'Show raw' : 'Beautify JSON') : 'Not valid JSON'}
                >
                  {beautified ? 'Raw' : 'Beautify'}
                </button>
              </div>
              {isJsonBeautified ? (
                <div className="diag-highlighted-block">
                  <SyntaxHighlighter
                    style={highlighterStyle}
                    language="json"
                    PreTag="div"
                    wrapLongLines={resultWrapped}
                    codeTagProps={{ style: { whiteSpace: resultWrapped ? 'pre-wrap' : 'pre' } }}
                    customStyle={{
                      margin: 0,
                      padding: '8px',
                      fontSize: '11px',
                      lineHeight: '1.5',
                      overflow: 'auto',
                      maxHeight: '320px',
                    }}
                  >
                    {displayResult ?? ''}
                  </SyntaxHighlighter>
                </div>
              ) : (
                <pre className="diag-result-code">{displayResult}</pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function LoopLimitEntry({ event }: { event: LoopLimitEvent }) {
  return (
    <div className="diag-entry diag-loop-limit">
      <div className="diag-entry-header">
        <div className="diag-entry-icon diag-icon-warn">
          <AlertTriangle size={14} />
        </div>
        <div className="diag-entry-main">
          <span className="diag-entry-title">Loop Limit Reached</span>
        </div>
        <div className="diag-entry-badges">
          <span className="diag-badge diag-badge-err">
            {event.iterations}/{event.max_iterations}
          </span>
        </div>
      </div>
    </div>
  );
}

export function DiagnosticsPanel({ entries, summary: _summary, isActive, isGlobal, expanded, onToggleExpand, onClear, onClose }: DiagnosticsPanelProps) {
  const endRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLDivElement>(null);
  const [filterOpen, setFilterOpen] = useState(false);
  const [timeRange, setTimeRange] = useState<TimeRange>('1h');

  const filteredEntries = useMemo(() => filterByTimeRange(entries, timeRange), [entries, timeRange]);
  const summary = useMemo(() => computeSummary(filteredEntries), [filteredEntries]);

  // Auto-scroll to bottom.
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [filteredEntries]);

  // Close filter popover on outside click.
  useEffect(() => {
    if (!filterOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (filterRef.current && !filterRef.current.contains(e.target as Node)) {
        setFilterOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [filterOpen]);

  const panelContent = (
    <div className={`diag-panel ${expanded ? 'diag-expanded' : ''}`}>
      <div className="diag-header">
        <div className="diag-header-left">
          <Activity size={16} className="diag-header-icon" />
          <h3 className="diag-title">Diagnostics{isGlobal && <span className="diag-global-label"> (Global)</span>}</h3>
          {isActive && <span className="diag-live-dot" />}
        </div>
        <div className="diag-header-actions">
          <div className="diag-filter-wrapper" ref={filterRef}>
            <button
              className={`diag-btn${timeRange !== 'all' ? ' diag-btn-active' : ''}`}
              onClick={() => setFilterOpen(!filterOpen)}
              title="Filter by time range"
            >
              <Filter size={14} />
            </button>
            {filterOpen && (
              <div className="diag-filter-popover">
                <div className="diag-filter-title">Time range</div>
                {TIME_RANGE_OPTIONS.map((opt) => (
                  <button
                    key={opt.value}
                    className={`diag-filter-option${timeRange === opt.value ? ' active' : ''}`}
                    onClick={() => {
                      setTimeRange(opt.value);
                      setFilterOpen(false);
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            )}
          </div>
          <button className="diag-btn" onClick={onToggleExpand} title={expanded ? 'Collapse' : 'Expand'}>
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </button>
          <button className="diag-btn" onClick={onClear} title="Clear">
            <Trash2 size={14} />
          </button>
          <button className="diag-btn" onClick={onClose} title="Close">
            <X size={14} />
          </button>
        </div>
      </div>

      {/* Summary bar */}
      {filteredEntries.length > 0 && (
        <div className="diag-summary">
          <div className="diag-summary-item">
            <span className="diag-summary-value">{summary.totalIterations}</span>
            <span className="diag-summary-label">iterations</span>
          </div>
          <div className="diag-summary-item">
            <span className="diag-summary-value">
              {(summary.totalInputTokens + summary.totalOutputTokens).toLocaleString()}
            </span>
            <span className="diag-summary-label">tokens</span>
          </div>
          <div className="diag-summary-item">
            <span className="diag-summary-value">${summary.totalCost.toFixed(4)}</span>
            <span className="diag-summary-label">cost</span>
          </div>
          <div className="diag-summary-item">
            <span className="diag-summary-value">
              {summary.toolCallCount > 0 ? `${summary.toolSuccessCount}/${summary.toolCallCount}` : '--'}
            </span>
            <span className="diag-summary-label">tools ok</span>
          </div>
        </div>
      )}

      {/* Timeline */}
      <div className="diag-timeline">
        {filteredEntries.length === 0 && entries.length === 0 && (
          <div className="diag-empty">
            <Activity size={24} className="diag-empty-icon" />
            <p className="diag-empty-text">No diagnostics data yet.</p>
            <p className="diag-empty-hint">Send a message to see real-time request details.</p>
          </div>
        )}
        {filteredEntries.length === 0 && entries.length > 0 && (
          <div className="diag-empty">
            <Filter size={24} className="diag-empty-icon" />
            <p className="diag-empty-text">No entries in selected time range.</p>
            <p className="diag-empty-hint">Try expanding the time range filter.</p>
          </div>
        )}
        {filteredEntries.map((entry) => {
          switch (entry.event.type) {
            case 'llm_response':
              return <LlmEntry key={entry.id} event={entry.event} timestamp={entry.timestamp} />;
            case 'tool_result':
              return <ToolEntry key={entry.id} event={entry.event} timestamp={entry.timestamp} />;
            case 'loop_limit_hit':
              return <LoopLimitEntry key={entry.id} event={entry.event} />;
            case 'user_message':
              return <UserMessageEntry key={entry.id} event={entry.event} timestamp={entry.timestamp} />;
            default:
              return null;
          }
        })}
        <div ref={endRef} />
      </div>
    </div>
  );

  if (expanded) {
    return (
      <div className="diag-backdrop" onClick={onClose}>
        <div onClick={(e) => e.stopPropagation()}>
          {panelContent}
        </div>
      </div>
    );
  }

  return panelContent;
}
