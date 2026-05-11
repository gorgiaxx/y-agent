/**
 * chatSearchHighlight.test.ts -- unit tests for the splitTextByQuery pure
 * function used by HighlightedText to split text at case-insensitive search
 * matches.
 */

import { describe, it, expect, beforeAll } from 'vitest';

// Delayed import so mocks (if any) can be installed before module load.
let splitTextByQuery: (
  text: string,
  query: string,
) => Array<{ text: string; isMatch: boolean }>;

beforeAll(async () => {
  ({ splitTextByQuery } = await import(
    '../components/chat-panel/chat-box/searchHighlightUtils'
  ));
});

describe('splitTextByQuery', () => {
  it('returns original text when query is empty', () => {
    const result = splitTextByQuery('Hello world', '');
    expect(result).toEqual([{ text: 'Hello world', isMatch: false }]);
  });

  it('returns original text when there is no match', () => {
    const result = splitTextByQuery('Hello world', 'xyz');
    expect(result).toEqual([{ text: 'Hello world', isMatch: false }]);
  });

  it('splits text at case-insensitive matches', () => {
    const result = splitTextByQuery('Hello World hello', 'hello');
    expect(result).toEqual([
      { text: 'Hello', isMatch: true },
      { text: ' World ', isMatch: false },
      { text: 'hello', isMatch: true },
    ]);
  });

  it('handles query at start of text', () => {
    const result = splitTextByQuery('foobar baz', 'foo');
    expect(result).toEqual([
      { text: 'foo', isMatch: true },
      { text: 'bar baz', isMatch: false },
    ]);
  });

  it('handles query at end of text', () => {
    const result = splitTextByQuery('baz foobar', 'bar');
    expect(result).toEqual([
      { text: 'baz foo', isMatch: false },
      { text: 'bar', isMatch: true },
    ]);
  });

  it('handles multiple consecutive matches', () => {
    const result = splitTextByQuery('aaa', 'a');
    expect(result).toEqual([
      { text: 'a', isMatch: true },
      { text: 'a', isMatch: true },
      { text: 'a', isMatch: true },
    ]);
  });

  it('escapes regex special characters in query', () => {
    const result = splitTextByQuery('price is $10.00 (USD)', '$10.00');
    expect(result).toEqual([
      { text: 'price is ', isMatch: false },
      { text: '$10.00', isMatch: true },
      { text: ' (USD)', isMatch: false },
    ]);
  });

  it('handles empty text input', () => {
    const result = splitTextByQuery('', 'hello');
    expect(result).toEqual([{ text: '', isMatch: false }]);
  });

  it('handles query longer than text', () => {
    const result = splitTextByQuery('hi', 'hello world');
    expect(result).toEqual([{ text: 'hi', isMatch: false }]);
  });

  it('handles entire text as a match', () => {
    const result = splitTextByQuery('hello', 'hello');
    expect(result).toEqual([{ text: 'hello', isMatch: true }]);
  });

  it('preserves original case in matched text', () => {
    const result = splitTextByQuery('HeLLo WoRLd', 'hello');
    expect(result).toEqual([
      { text: 'HeLLo', isMatch: true },
      { text: ' WoRLd', isMatch: false },
    ]);
  });
});
