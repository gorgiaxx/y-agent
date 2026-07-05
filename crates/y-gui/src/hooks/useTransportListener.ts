// ---------------------------------------------------------------------------
// useTransportListener -- subscribe to a transport event with correct cleanup.
//
// `transport.listen()` returns `Promise<UnlistenFn>` (genuinely async in Tauri
// mode due to the dynamic `import('@tauri-apps/api/event')`). The naive
// `let unlisten; ...then((fn) => { unlisten = fn; }); return () => unlisten?.()`
// pattern leaks the listener if the component unmounts before the promise
// resolves: cleanup runs with `undefined` and the listener is never removed.
//
// This hook captures the promise and awaits it in cleanup, so the unlisten
// always runs. The callback is held in a ref so the listener is registered once
// (per `deps` change) but always invokes the latest closure -- no stale values.
// ---------------------------------------------------------------------------

import { useEffect, useRef } from 'react';
import { transport } from '../lib';

export function useTransportListener<T>(
  event: string,
  callback: (event: { payload: T }) => void,
  deps: React.DependencyList,
): void {
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  useEffect(() => {
    const unlisten = transport.listen<T>(event, (e) => {
      callbackRef.current(e);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
    // deps are intentionally spread; the listener re-registers when they change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [event, ...deps]);
}
