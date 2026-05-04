import { lazy, Suspense, type CSSProperties } from 'react';

import type { MonacoEditorProps } from './MonacoEditorCore';

const MonacoEditorCore = lazy(() =>
  import('./MonacoEditorCore').then((module) => ({
    default: module.MonacoEditorCore,
  })),
);

function loadingStyle(height?: string | number, width?: string | number): CSSProperties {
  const style: CSSProperties = {};
  if (height != null) style.height = height;
  if (width != null) style.width = width;
  return style;
}

export function MonacoEditor(props: MonacoEditorProps) {
  return (
    <Suspense
      fallback={
        <div
          className={props.className}
          style={loadingStyle(props.height, props.width)}
        />
      }
    >
      <MonacoEditorCore {...props} />
    </Suspense>
  );
}

MonacoEditor.displayName = 'MonacoEditor';

export type { MonacoEditorProps };
