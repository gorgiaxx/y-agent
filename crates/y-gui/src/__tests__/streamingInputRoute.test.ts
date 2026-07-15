import { describe, expect, it } from 'vitest';

import { routeStreamingInput } from '../components/chat-panel/input-area/streamingInputRoute';

describe('routeStreamingInput', () => {
  it('keeps plain input on the normal message route', () => {
    expect(routeStreamingInput('  focus on the parser  ')).toEqual({
      kind: 'message',
      text: 'focus on the parser',
    });
  });

  it('routes an explicit todo command without its slash prefix', () => {
    expect(routeStreamingInput('/todo  run the release checks  ')).toEqual({
      kind: 'todo',
      text: 'run the release checks',
    });
  });

  it('does not mistake a longer slash command for todo', () => {
    expect(routeStreamingInput('/todos are not todo commands')).toEqual({
      kind: 'message',
      text: '/todos are not todo commands',
    });
  });
});
