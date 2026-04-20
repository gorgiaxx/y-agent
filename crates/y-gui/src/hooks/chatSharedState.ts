// ---------------------------------------------------------------------------
// Shared mutable refs passed between decomposed chat sub-hooks.
//
// The refs are created once in useChat (the facade) and threaded into each
// sub-hook so they share the same underlying Maps / arrays. React state
// setters are passed separately since they belong to the hook that owns the
// visible slice.
// ---------------------------------------------------------------------------

import type { MutableRefObject } from 'react';
import type { Message } from '../types';
import type { ToolResultRecord } from './chatStreamTypes';
import type { InterleavedSegment } from './useInterleavedSegments';
import type { ChatOpStatus, CompactInfo } from './useChat';

export interface ChatSharedRefs {
  activeSessionIdRef: MutableRefObject<string | null>;
  sessionMessagesRef: MutableRefObject<Map<string, Message[]>>;
  sessionActivityRef: MutableRefObject<Map<string, number>>;
  opStatusMapRef: MutableRefObject<Map<string, ChatOpStatus>>;
  opStatusRef: MutableRefObject<ChatOpStatus>;
  toolResultsRef: MutableRefObject<Map<string, ToolResultRecord[]>>;
  streamSegsRef: MutableRefObject<Map<string, InterleavedSegment[]>>;
  contextResetMapRef: MutableRefObject<Map<string, number[]>>;
  compactMapRef: MutableRefObject<Map<string, CompactInfo[]>>;
  rootAgentNamesRef: MutableRefObject<string[]>;
}
