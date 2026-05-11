import { createContext } from 'react';
import type { ChatSearchState } from '../../hooks/useChatSearch';

export const ChatSearchContext = createContext<ChatSearchState | null>(null);
