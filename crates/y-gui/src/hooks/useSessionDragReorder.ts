import { useState, useRef, useCallback, useEffect } from 'react';
import type { SessionInfo } from '../types';

interface UseSessionDragReorderOptions {
  sessions: SessionInfo[];
  storageKey: string;
}

export function useSessionDragReorder({ sessions, storageKey }: UseSessionDragReorderOptions) {
  const [draggedSessionId, setDraggedSessionId] = useState<string | null>(null);
  const [dragOverSessionId, setDragOverSessionId] = useState<string | null>(null);
  const [dragOverPosition, setDragOverPosition] = useState<'above' | 'below'>('above');
  const dragGroupRef = useRef<string[]>([]);
  const dropTargetRef = useRef<{ targetId: string; position: 'above' | 'below' } | null>(null);

  const [sessionOrder, setSessionOrder] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem(storageKey);
      if (stored) {
        const parsed = JSON.parse(stored) as string[];
        if (Array.isArray(parsed)) return parsed;
      }
    } catch { /* ignore */ }
    return [];
  });

  useEffect(() => {
    localStorage.setItem(storageKey, JSON.stringify(sessionOrder));
  }, [storageKey, sessionOrder]);

  useEffect(() => {
    return () => { document.body.classList.remove('y-gui-dragging'); };
  }, []);

  const commitReorder = useCallback(
    (sourceId: string, targetId: string, dropPos: 'above' | 'below', groupSessionIds: string[]) => {
      if (sourceId === targetId) return;
      if (!groupSessionIds.includes(sourceId)) return;

      const newGroupOrder = groupSessionIds.filter((id) => id !== sourceId);
      const targetIdx = newGroupOrder.indexOf(targetId);
      if (targetIdx === -1) return;
      const insertIdx = dropPos === 'below' ? targetIdx + 1 : targetIdx;
      newGroupOrder.splice(insertIdx, 0, sourceId);

      const allIds = sessions.map((s) => s.id);
      const currentOrder = sessionOrder.length > 0
        ? [...sessionOrder, ...allIds.filter((id) => !sessionOrder.includes(id))]
        : [...allIds];
      const groupSet = new Set(groupSessionIds);
      const firstGroupPos = currentOrder.findIndex((id) => groupSet.has(id));
      const withoutGroup = currentOrder.filter((id) => !groupSet.has(id));
      withoutGroup.splice(firstGroupPos, 0, ...newGroupOrder);
      setSessionOrder(withoutGroup);
    },
    [sessions, sessionOrder],
  );

  const handleItemHover = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      if (!draggedSessionId || draggedSessionId === sessionId) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const pos: 'above' | 'below' = e.clientY < rect.top + rect.height / 2 ? 'above' : 'below';
      setDragOverSessionId(sessionId);
      setDragOverPosition(pos);
      dropTargetRef.current = { targetId: sessionId, position: pos };
    },
    [draggedSessionId],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent, sessionId: string, groupSessionIds: string[], ignoreSelector = '.button--icon') => {
      if (e.button !== 0) return;
      if ((e.target as HTMLElement).closest(ignoreSelector)) return;

      const startX = e.clientX;
      const startY = e.clientY;
      let dragging = false;

      const onMove = (me: MouseEvent) => {
        if (!dragging) {
          const dx = me.clientX - startX;
          const dy = me.clientY - startY;
          if (Math.abs(dx) + Math.abs(dy) < 4) return;
          dragging = true;
          dragGroupRef.current = groupSessionIds;
          dropTargetRef.current = null;
          setDraggedSessionId(sessionId);
          document.body.classList.add('y-gui-dragging');
        }
      };

      const onUp = () => {
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        document.body.classList.remove('y-gui-dragging');
        if (!dragging) return;

        const target = dropTargetRef.current;
        if (target) {
          commitReorder(sessionId, target.targetId, target.position, dragGroupRef.current);
        }

        dropTargetRef.current = null;
        setDraggedSessionId(null);
        setDragOverSessionId(null);
      };

      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    },
    [commitReorder],
  );

  const getPreviewList = useCallback(
    (list: SessionInfo[]) => {
      if (!draggedSessionId || !dragOverSessionId || draggedSessionId === dragOverSessionId) {
        return list;
      }
      const sourceIdx = list.findIndex((s) => s.id === draggedSessionId);
      const targetIdx = list.findIndex((s) => s.id === dragOverSessionId);
      if (sourceIdx === -1 || targetIdx === -1) return list;

      const result = [...list];
      const [sourceItem] = result.splice(sourceIdx, 1);
      const newTargetIdx = result.findIndex((s) => s.id === dragOverSessionId);
      const insertAt = dragOverPosition === 'below' ? newTargetIdx + 1 : newTargetIdx;
      result.splice(insertAt, 0, sourceItem);
      return result;
    },
    [draggedSessionId, dragOverSessionId, dragOverPosition],
  );

  const sortByUserOrder = useCallback(
    (list: SessionInfo[]): SessionInfo[] => {
      if (sessionOrder.length === 0) return list;
      const orderMap = new Map(sessionOrder.map((id, idx) => [id, idx]));
      return [...list].sort((a, b) => {
        const ia = orderMap.get(a.id);
        const ib = orderMap.get(b.id);
        if (ia !== undefined && ib !== undefined) return ia - ib;
        if (ia === undefined && ib === undefined) {
          return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
        }
        return ia === undefined ? -1 : 1;
      });
    },
    [sessionOrder],
  );

  return {
    draggedSessionId,
    dragOverSessionId,
    dragOverPosition,
    sessionOrder,
    commitReorder,
    handleItemHover,
    handleMouseDown,
    getPreviewList,
    sortByUserOrder,
  };
}
