// ---------------------------------------------------------------------------
// Unit tests for tomlUtils: serializeToml, deserializeFromJson, mergeIntoRawToml
// ---------------------------------------------------------------------------

import { describe, it, expect } from 'vitest';
import {
  serializeToml,
  deserializeFromJson,
  mergeIntoRawToml,
  escapeTomlString,
  type FieldDef,
} from '../utils/tomlUtils';

// ---------------------------------------------------------------------------
// escapeTomlString
// ---------------------------------------------------------------------------

describe('escapeTomlString', () => {
  it('escapes backslashes', () => {
    expect(escapeTomlString('C:\\Program Files\\app')).toBe('C:\\\\Program Files\\\\app');
  });

  it('escapes double quotes', () => {
    expect(escapeTomlString('say "hello"')).toBe('say \\"hello\\"');
  });

  it('returns unchanged string when no special chars', () => {
    expect(escapeTomlString('normal')).toBe('normal');
  });
});

// ---------------------------------------------------------------------------
// serializeToml
// ---------------------------------------------------------------------------

describe('serializeToml', () => {
  const schema: FieldDef[] = [
    { formKey: 'name', tomlKey: 'name', type: 'string', defaultValue: '' },
    { formKey: 'count', tomlKey: 'count', type: 'number', defaultValue: 0 },
    { formKey: 'enabled', tomlKey: 'enabled', type: 'boolean', defaultValue: false },
    { formKey: 'tags', tomlKey: 'tags', type: 'string[]', defaultValue: [] },
  ];

  it('serializes all field types correctly', () => {
    const data = { name: 'test', count: 42, enabled: true, tags: ['a', 'b'] };
    const toml = serializeToml(data, schema);
    expect(toml).toContain('name = "test"');
    expect(toml).toContain('count = 42');
    expect(toml).toContain('enabled = true');
    expect(toml).toContain('tags = ["a", "b"]');
  });

  it('omits optional fields when empty', () => {
    const optSchema: FieldDef[] = [
      { formKey: 'url', tomlKey: 'url', type: 'string', defaultValue: '', optional: true },
      { formKey: 'name', tomlKey: 'name', type: 'string', defaultValue: '' },
    ];
    const data = { url: '', name: 'hello' };
    const toml = serializeToml(data, optSchema);
    expect(toml).not.toContain('url');
    expect(toml).toContain('name = "hello"');
  });

  it('writes section headers for nested fields', () => {
    const nestedSchema: FieldDef[] = [
      { formKey: 'maxRetries', tomlKey: 'max_retries', section: 'retry', type: 'number', defaultValue: 3 },
    ];
    const data = { maxRetries: 5 };
    const toml = serializeToml(data, nestedSchema);
    expect(toml).toContain('[retry]');
    expect(toml).toContain('max_retries = 5');
  });
});

// ---------------------------------------------------------------------------
// deserializeFromJson
// ---------------------------------------------------------------------------

describe('deserializeFromJson', () => {
  it('extracts values from nested JSON objects', () => {
    const schema: FieldDef[] = [
      { formKey: 'pruningEnabled', tomlKey: 'enabled', section: 'pruning', type: 'boolean', defaultValue: false },
      { formKey: 'rootField', tomlKey: 'name', type: 'string', defaultValue: '' },
    ];
    const json = { name: 'test', pruning: { enabled: true } };
    const result = deserializeFromJson(json, schema);
    expect(result.pruningEnabled).toBe(true);
    expect(result.rootField).toBe('test');
  });

  it('applies default values for missing fields', () => {
    const schema: FieldDef[] = [
      { formKey: 'timeout', tomlKey: 'timeout', type: 'number', defaultValue: 30 },
    ];
    const result = deserializeFromJson({}, schema);
    expect(result.timeout).toBe(30);
  });

  it('handles string[] type correctly', () => {
    const schema: FieldDef[] = [
      { formKey: 'tags', tomlKey: 'tags', type: 'string[]', defaultValue: [] },
    ];
    const result = deserializeFromJson({ tags: ['a', 'b', 'c'] }, schema);
    expect(result.tags).toEqual(['a', 'b', 'c']);
  });
});

// ---------------------------------------------------------------------------
// mergeIntoRawToml
// ---------------------------------------------------------------------------

describe('mergeIntoRawToml', () => {
  const schema: FieldDef[] = [
    { formKey: 'name', tomlKey: 'name', type: 'string', defaultValue: '' },
    { formKey: 'count', tomlKey: 'count', type: 'number', defaultValue: 0 },
  ];

  it('preserves comments while updating values', () => {
    const raw = `# This is a comment\nname = "old"\n# Another comment\ncount = 10\n`;
    const data = { name: 'new', count: 20 };
    const result = mergeIntoRawToml(raw, data, schema);
    expect(result).toContain('# This is a comment');
    expect(result).toContain('# Another comment');
    expect(result).toContain('name = "new"');
    expect(result).toContain('count = 20');
    expect(result).not.toContain('name = "old"');
  });

  it('appends missing fields at the end', () => {
    const raw = `name = "existing"\n`;
    const data = { name: 'existing', count: 42 };
    const result = mergeIntoRawToml(raw, data, schema);
    expect(result).toContain('name = "existing"');
    expect(result).toContain('count = 42');
  });

  it('falls back to serializeToml when raw is empty', () => {
    const data = { name: 'test', count: 5 };
    const result = mergeIntoRawToml('', data, schema);
    expect(result).toContain('name = "test"');
    expect(result).toContain('count = 5');
  });

  it('preserves blank lines', () => {
    const raw = `name = "a"\n\ncount = 1\n`;
    const data = { name: 'b', count: 2 };
    const result = mergeIntoRawToml(raw, data, schema);
    // Should have a blank line between the two fields
    expect(result).toMatch(/name = "b"\n\ncount = 2/);
  });

  // ---------------------------------------------------------------------------
  // Section-aware placement of missing fields.
  //
  // Regression: a schema field whose section header already exists but whose
  // key is absent from the raw text (e.g. it is commented out) must be written
  // *inside* that section, not appended at the end of the file -- otherwise it
  // lands under whichever section happens to be last, corrupting the config.
  // ---------------------------------------------------------------------------

  const sectionedSchema: FieldDef[] = [
    { formKey: 'loopMax', tomlKey: 'max_iterations', section: 'loop_guard', type: 'number', defaultValue: 50, optional: true },
    { formKey: 'hitlAuto', tomlKey: 'auto_approve', section: 'hitl', type: 'boolean', defaultValue: true, optional: true },
  ];

  it('writes a missing field into its existing section, not under the last one', () => {
    const raw = [
      '[loop_guard]',
      '# max_iterations = 50',
      '',
      '[hitl]',
      '# auto_approve = true',
      '',
    ].join('\n');
    const data = { loopMax: 99, hitlAuto: true };
    const result = mergeIntoRawToml(raw, data, sectionedSchema);

    // max_iterations belongs under [loop_guard], before [hitl].
    const loopIdx = result.indexOf('[loop_guard]');
    const hitlIdx = result.indexOf('[hitl]');
    const valIdx = result.indexOf('max_iterations = 99');
    expect(valIdx).toBeGreaterThan(loopIdx);
    expect(valIdx).toBeLessThan(hitlIdx);
  });

  it('does not place a loop_guard field under [hitl]', () => {
    const raw = `[loop_guard]\n# max_iterations = 50\n\n[hitl]\n# auto_approve = true\n`;
    const data = { loopMax: 99, hitlAuto: true };
    const result = mergeIntoRawToml(raw, data, sectionedSchema);
    const hitlBlock = result.slice(result.indexOf('[hitl]'));
    expect(hitlBlock).not.toContain('max_iterations');
  });

  it('places a missing root-level field above section headers', () => {
    const rootSchema: FieldDef[] = [
      { formKey: 'name', tomlKey: 'name', type: 'string', defaultValue: '' },
      { formKey: 'nested', tomlKey: 'flag', section: 'sub', type: 'boolean', defaultValue: false },
    ];
    const raw = `[sub]\nflag = true\n`;
    const data = { name: 'top', nested: true };
    const result = mergeIntoRawToml(raw, data, rootSchema);
    expect(result.indexOf('name = "top"')).toBeLessThan(result.indexOf('[sub]'));
  });

  it('avoids duplicate keys across repeated saves', () => {
    const raw = `[loop_guard]\n# max_iterations = 50\n\n[hitl]\n# auto_approve = true\n`;
    const data = { loopMax: 99, hitlAuto: true };
    const first = mergeIntoRawToml(raw, data, sectionedSchema);
    const second = mergeIntoRawToml(first, { loopMax: 88, hitlAuto: true }, sectionedSchema);
    const occurrences = second.split('\n').filter((l) => l.trim().startsWith('max_iterations')).length;
    expect(occurrences).toBe(1);
    expect(second).toContain('max_iterations = 88');
  });
});
