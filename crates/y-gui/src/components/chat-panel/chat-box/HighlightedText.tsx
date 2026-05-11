import React, { type ReactNode } from 'react';
import { useChatSearchContext } from '../../../hooks/useChatSearchContext';
import { splitTextByQuery } from './searchHighlightUtils';

function highlightChildren(children: ReactNode, query: string): ReactNode {
  return React.Children.map(children, (child) => {
    if (typeof child === 'string') {
      const segments = splitTextByQuery(child, query);
      if (segments.length === 1 && !segments[0].isMatch) return child;
      return segments.map((seg, i) =>
        seg.isMatch ? (
          <mark key={i} className="chat-search-match" data-search-match>
            {seg.text}
          </mark>
        ) : (
          seg.text
        ),
      );
    }

    if (typeof child === 'number') {
      return highlightChildren(String(child), query);
    }

    if (React.isValidElement(child)) {
      const props = child.props as { children?: ReactNode };
      if (props.children != null) {
        return React.cloneElement(
          child,
          undefined,
          highlightChildren(props.children, query),
        );
      }
    }

    return child;
  });
}

export function HighlightedText({ children }: { children: ReactNode }) {
  const ctx = useChatSearchContext();
  if (!ctx.query) return <>{children}</>;
  return <>{highlightChildren(children, ctx.query)}</>;
}
