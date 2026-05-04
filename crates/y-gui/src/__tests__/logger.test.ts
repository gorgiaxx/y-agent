import { describe, expect, it, vi } from 'vitest';

import { createLogger } from '../lib/logger';

describe('logger', () => {
  it('suppresses debug and info below the configured level', () => {
    const sink = {
      debug: vi.fn(),
      info: vi.fn(),
      warn: vi.fn(),
      error: vi.fn(),
    };

    const logger = createLogger(sink, 'warn');
    logger.debug('debug detail');
    logger.info('status detail');
    logger.warn('warning detail');
    logger.error('error detail');

    expect(sink.debug).not.toHaveBeenCalled();
    expect(sink.info).not.toHaveBeenCalled();
    expect(sink.warn).toHaveBeenCalledWith('warning detail');
    expect(sink.error).toHaveBeenCalledWith('error detail');
  });

  it('can be silenced for tests and production embeddings', () => {
    const sink = {
      debug: vi.fn(),
      info: vi.fn(),
      warn: vi.fn(),
      error: vi.fn(),
    };

    const logger = createLogger(sink, 'silent');
    logger.warn('warning detail');
    logger.error('error detail');

    expect(sink.warn).not.toHaveBeenCalled();
    expect(sink.error).not.toHaveBeenCalled();
  });
});
