import { transport } from './index';

export type AssistantFeedbackRating = 'good' | 'bad';

export interface AssistantFeedbackInput {
  traceId: string;
  feedbackId: string;
  rating: AssistantFeedbackRating;
  comment?: string;
}

export async function submitAssistantFeedback(input: AssistantFeedbackInput): Promise<void> {
  const traceId = input.traceId.trim();
  const feedbackId = input.feedbackId.trim();
  const comment = input.comment?.trim();
  if (!traceId || !feedbackId) {
    throw new Error('Feedback requires trace and feedback identifiers.');
  }
  if (input.rating === 'bad' && !comment) {
    throw new Error('Negative feedback requires an actionable correction.');
  }

  await transport.invoke('chat_feedback', {
    traceId,
    feedbackId,
    score: input.rating === 'good' ? 1 : 0,
    comment,
  });
}

export function createFeedbackId(): string {
  const bytes = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
