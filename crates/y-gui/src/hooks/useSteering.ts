// useSteering -- per-session steering queue state + transport actions.
//
// The backend is the source of truth. This hook keeps an optimistic mirror,
// reconciled against the `chat:steer_queue` broadcast (add/delete) and the
// `steer_injected` event (drained at an LLM-call boundary).

import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { transport, logger } from '../lib';
import type { SteerMessage } from '../types';
import { chatBusSubscribers, type ChatBusEvent } from './chatBus';
import {
  type SteeringQueues,
  createSteeringQueues,
  getQueue,
  setQueue,
  addSteer as addToQueue,
  removeSteer as removeFromQueue,
} from './steeringState';

export interface UseSteeringReturn {
  /** Pending steers for a session (FIFO), reactive to queue changes. */
  steersFor: (sessionId: string | null) => SteerMessage[];
  /** Enqueue a steering message for injection at the next LLM-call boundary. */
  addSteer: (sessionId: string, text: string) => Promise<void>;
  /** Remove a pending steering message by id. */
  deleteSteer: (sessionId: string, steerId: string) => Promise<void>;
  /** Take and clear a session's pending steers (used for residual replay). */
  popResiduals: (sessionId: string) => SteerMessage[];
}

export function useSteering(): UseSteeringReturn {
  const [queues, setQueues] = useState<SteeringQueues>(createSteeringQueues);
  // Latest committed snapshot for synchronous reads (popResiduals from effects).
  const queuesRef = useRef<SteeringQueues>(queues);
  useEffect(() => {
    queuesRef.current = queues;
  }, [queues]);

  useEffect(() => {
    const handler = (event: ChatBusEvent) => {
      if (event.type === 'steer_queue') {
        setQueues((prev) => setQueue(prev, event.session_id, event.queue));
      } else if (event.type === 'steer_injected') {
        setQueues((prev) => removeFromQueue(prev, event.session_id, event.steer_id));
      }
    };
    chatBusSubscribers.add(handler);
    return () => {
      chatBusSubscribers.delete(handler);
    };
  }, []);

  const steersFor = useCallback(
    (sessionId: string | null) => (sessionId ? getQueue(queues, sessionId) : []),
    [queues],
  );

  const addSteer = useCallback(async (sessionId: string, text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    try {
      const steer = await transport.invoke<SteerMessage>('chat_add_steer', {
        sessionId,
        text: trimmed,
      });
      setQueues((prev) => addToQueue(prev, sessionId, steer));
    } catch (e) {
      logger.error('[useSteering] add steer failed:', e);
    }
  }, []);

  const deleteSteer = useCallback(async (sessionId: string, steerId: string) => {
    // Optimistic removal for a snappy UI; the broadcast reconciles.
    setQueues((prev) => removeFromQueue(prev, sessionId, steerId));
    try {
      await transport.invoke('chat_delete_steer', { sessionId, steerId });
    } catch (e) {
      logger.error('[useSteering] delete steer failed:', e);
    }
  }, []);

  const popResiduals = useCallback((sessionId: string): SteerMessage[] => {
    const residuals = getQueue(queuesRef.current, sessionId);
    if (residuals.length > 0) {
      setQueues((prev) => setQueue(prev, sessionId, []));
    }
    return residuals;
  }, []);

  return useMemo(
    () => ({ steersFor, addSteer, deleteSteer, popResiduals }),
    [steersFor, addSteer, deleteSteer, popResiduals],
  );
}
