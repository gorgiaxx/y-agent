export interface ContextResetRemoval {
  points: number[];
  persistedIndex: number | null;
}

export function removeContextResetPoint(
  points: number[],
  pointIndex: number,
): ContextResetRemoval {
  if (pointIndex < 0 || pointIndex >= points.length) {
    return {
      points,
      persistedIndex: points.at(-1) ?? null,
    };
  }

  const updated = points.filter((_, index) => index !== pointIndex);
  return {
    points: updated,
    persistedIndex: updated.at(-1) ?? null,
  };
}
