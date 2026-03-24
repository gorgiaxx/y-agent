import { useState, useEffect, useRef, useCallback } from 'react';
import mermaid from 'mermaid';
import { Copy, Check } from 'lucide-react';
import { useResolvedTheme } from '../../../hooks/useTheme';
import './MermaidBlock.css';

/** Monotonically increasing counter to generate unique render IDs. */
let mermaidIdCounter = 0;

/** Track whether mermaid has been initialized and with which theme. */
let currentMermaidTheme: string | null = null;

/**
 * Initialize (or re-initialize) mermaid with the given theme.
 * Only re-initializes when the theme actually changes.
 */
function ensureMermaidInit(resolvedTheme: string) {
  const mermaidTheme = resolvedTheme === 'dark' ? 'dark' : 'default';
  if (currentMermaidTheme === mermaidTheme) return;
  mermaid.initialize({
    startOnLoad: false,
    theme: mermaidTheme,
    securityLevel: 'strict',
  });
  currentMermaidTheme = mermaidTheme;
}

interface MermaidBlockProps {
  code: string;
}

/**
 * Renders a mermaid diagram from source code.
 *
 * - Shows a loading skeleton while the diagram is being rendered.
 * - Displays the rendered SVG on success.
 * - Falls back to a styled error message with the raw code on failure.
 * - Provides a "Copy code" button to copy the raw mermaid source.
 */
export function MermaidBlock({ code }: MermaidBlockProps) {
  const resolvedTheme = useResolvedTheme();
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Unique ID for this render instance.
  const renderIdRef = useRef<string>('');
  if (!renderIdRef.current) {
    renderIdRef.current = `mermaid-${++mermaidIdCounter}`;
  }

  useEffect(() => {
    let cancelled = false;

    async function renderDiagram() {
      try {
        ensureMermaidInit(resolvedTheme);
        const { svg: renderedSvg } = await mermaid.render(
          renderIdRef.current,
          code,
        );
        if (!cancelled) {
          setSvg(renderedSvg);
          setError(null);
        }
      } catch (err: unknown) {
        if (!cancelled) {
          setSvg(null);
          const msg = err instanceof Error ? err.message : String(err);
          setError(msg);
        }
        // mermaid.render inserts a temporary element on error; clean it up.
        const tempEl = document.getElementById(renderIdRef.current);
        if (tempEl) tempEl.remove();
      }
    }

    setSvg(null);
    setError(null);
    renderDiagram();

    return () => {
      cancelled = true;
    };
  }, [code, resolvedTheme]);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [code]);

  return (
    <div className="mermaid-block-wrapper">
      <div className="mermaid-block-header">
        <span className="mermaid-block-label">mermaid</span>
        <button
          className="code-block-copy"
          onClick={handleCopy}
          title="Copy code"
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <div className="mermaid-block-body" ref={containerRef}>
        {svg ? (
          <div
            className="mermaid-block-svg"
            dangerouslySetInnerHTML={{ __html: svg }}
          />
        ) : error ? (
          <div className="mermaid-block-error">
            <span className="mermaid-block-error-label">
              Diagram render failed
            </span>
            <pre className="mermaid-block-error-code">{code}</pre>
          </div>
        ) : (
          <div className="mermaid-block-loading">
            <div className="mermaid-block-skeleton" />
          </div>
        )}
      </div>
    </div>
  );
}
