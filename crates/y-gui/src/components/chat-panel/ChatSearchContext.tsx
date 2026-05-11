import type { ReactNode } from 'react';
import { useChatSearch } from '../../hooks/useChatSearch';
import { ChatSearchContext } from './chatSearchState';

export function ChatSearchProvider({ children }: { children: ReactNode }) {
  const state = useChatSearch();
  return (
    <ChatSearchContext.Provider value={state}>
      {children}
    </ChatSearchContext.Provider>
  );
}
