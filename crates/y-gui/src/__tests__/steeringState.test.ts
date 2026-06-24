import { describe, expect, it } from 'vitest';

import {
  createSteeringQueues,
  getQueue,
  setQueue,
  addSteer,
  removeSteer,
} from '../hooks/steeringState';
import type { SteerMessage } from '../types';

function steer(id: string, text = id): SteerMessage {
  return { id, text, created_at: 0 };
}

describe('steeringState', () => {
  it('adds steers in FIFO order and is idempotent by id', () => {
    let state = createSteeringQueues();
    state = addSteer(state, 's1', steer('a'));
    state = addSteer(state, 's1', steer('b'));
    // Duplicate id is ignored.
    state = addSteer(state, 's1', steer('a'));

    const queue = getQueue(state, 's1');
    expect(queue.map((s) => s.id)).toEqual(['a', 'b']);
  });

  it('isolates queues per session', () => {
    let state = createSteeringQueues();
    state = addSteer(state, 's1', steer('a'));
    state = addSteer(state, 's2', steer('x'));

    expect(getQueue(state, 's1').map((s) => s.id)).toEqual(['a']);
    expect(getQueue(state, 's2').map((s) => s.id)).toEqual(['x']);
  });

  it('removes a steer by id and drops the session key when empty', () => {
    let state = createSteeringQueues();
    state = addSteer(state, 's1', steer('a'));
    state = removeSteer(state, 's1', 'a');

    expect(getQueue(state, 's1')).toEqual([]);
    expect('s1' in state).toBe(false);
  });

  it('removeSteer is a no-op for an unknown id (same reference)', () => {
    let state = createSteeringQueues();
    state = addSteer(state, 's1', steer('a'));
    const next = removeSteer(state, 's1', 'missing');
    expect(next).toBe(state);
  });

  it('setQueue replaces the authoritative list and clears on empty', () => {
    let state = createSteeringQueues();
    state = addSteer(state, 's1', steer('a'));

    state = setQueue(state, 's1', [steer('b'), steer('c')]);
    expect(getQueue(state, 's1').map((s) => s.id)).toEqual(['b', 'c']);

    state = setQueue(state, 's1', []);
    expect('s1' in state).toBe(false);
  });
});
