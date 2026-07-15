import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

function readSource(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), 'utf8');
}

describe('active-run TODO flow', () => {
  it('routes active-run input to TODO and exposes steer only on TODO items', () => {
    const chatView = readSource('../views/ChatView.tsx');
    const inputArea = readSource('../components/chat-panel/input-area/InputArea.tsx');
    const todoQueueHook = readSource('../hooks/useTodoQueue.ts');

    expect(chatView).toContain('runActive={runActive}');
    expect(chatView).toContain('onTodo={(text) => { void handleTodo(text); }}');
    expect(chatView).toContain('onSteer={handleTodoSteer}');
    expect(chatView).toContain('onUndoSteer={handleTodoUndoSteer}');
    expect(chatView).not.toContain("toast('TODO will steer the next step', 'success')");
    expect(chatView).not.toContain('SteeringQueue');
    expect(inputArea).toContain('if (runActive)');
    expect(inputArea).toContain('onTodo?.(route.text)');
    expect(inputArea).not.toContain('onSteer?.(route.text)');
    expect(todoQueueHook).toContain("event.type === 'steer_injected'");
    expect(todoQueueHook).toContain(
      'removeFromQueue(prev, event.session_id, event.steer_id)',
    );
  });
});
