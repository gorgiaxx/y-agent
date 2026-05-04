import { useEffect, useState, type Dispatch, type SetStateAction } from 'react';

interface PersistentStateOptions<T> {
  deserialize?: (raw: string) => T | null;
  serialize?: (value: T) => string;
}

export function usePersistentState<T>(
  storageKey: string,
  defaultValue: T | (() => T),
  options: PersistentStateOptions<T> = {},
): [T, Dispatch<SetStateAction<T>>] {
  const { deserialize, serialize } = options;
  const [value, setValue] = useState<T>(() => {
    const fallback = typeof defaultValue === 'function'
      ? (defaultValue as () => T)()
      : defaultValue;

    try {
      if (typeof localStorage === 'undefined') return fallback;
      const stored = localStorage.getItem(storageKey);
      if (!stored) return fallback;
      return deserialize?.(stored) ?? (JSON.parse(stored) as T);
    } catch {
      return fallback;
    }
  });

  useEffect(() => {
    try {
      if (typeof localStorage === 'undefined') return;
      const serialized = serialize?.(value) ?? JSON.stringify(value);
      localStorage.setItem(storageKey, serialized);
    } catch {
      // Ignore unavailable storage or corrupt serializer output.
    }
  }, [serialize, storageKey, value]);

  return [value, setValue];
}
