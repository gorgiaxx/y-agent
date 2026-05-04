export type LogLevel = 'silent' | 'error' | 'warn' | 'info' | 'debug';

export interface LogSink {
  debug: (...args: unknown[]) => void;
  info: (...args: unknown[]) => void;
  warn: (...args: unknown[]) => void;
  error: (...args: unknown[]) => void;
}

export type Logger = LogSink;

const LOG_LEVELS: Record<LogLevel, number> = {
  silent: 0,
  error: 1,
  warn: 2,
  info: 3,
  debug: 4,
};

function normalizeLogLevel(value: unknown): LogLevel | null {
  if (typeof value !== 'string') return null;
  if (value === 'silent' || value === 'error' || value === 'warn' || value === 'info' || value === 'debug') {
    return value;
  }
  return null;
}

function defaultLogLevel(): LogLevel {
  const configured = normalizeLogLevel(import.meta.env.VITE_LOG_LEVEL);
  if (configured) return configured;
  return import.meta.env.DEV ? 'debug' : 'warn';
}

function canLog(configuredLevel: LogLevel, messageLevel: Exclude<LogLevel, 'silent'>): boolean {
  return LOG_LEVELS[configuredLevel] >= LOG_LEVELS[messageLevel];
}

export function createLogger(
  sink: LogSink = console,
  level: LogLevel = defaultLogLevel(),
): Logger {
  return {
    debug: (...args: unknown[]) => {
      if (canLog(level, 'debug')) sink.debug(...args);
    },
    info: (...args: unknown[]) => {
      if (canLog(level, 'info')) sink.info(...args);
    },
    warn: (...args: unknown[]) => {
      if (canLog(level, 'warn')) sink.warn(...args);
    },
    error: (...args: unknown[]) => {
      if (canLog(level, 'error')) sink.error(...args);
    },
  };
}

export const logger = createLogger();
