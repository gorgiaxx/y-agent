import { useContext } from 'react';
import { ChatSearchContext } from '../components/chat-panel/chatSearchState';
import type { ChatSearchState } from './useChatSearch';

const FALLBACK: ChatSearchState = {
  isOpen: false,
  query: '',
  rawQuery: '',
  currentIndex: 0,
  totalMatches: 0,
  setTotalMatches: () => {},
  goToNext: () => {},
  goToPrev: () => {},
  open: () => {},
  close: () => {},
  setQuery: () => {},
  inputRef: { current: null },
};

export function useChatSearchContext(): ChatSearchState {
  const ctx = useContext(ChatSearchContext);
  return ctx ?? FALLBACK;
}
