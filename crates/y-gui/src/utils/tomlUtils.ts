// ---------------------------------------------------------------------------
// Generic schema-driven TOML serializer / JSON deserializer.
//
// Instead of writing per-section `fooToToml` / `jsonToFoo` functions,
// define a FieldDef[] schema and call serializeToml / deserializeFromJson.
// ---------------------------------------------------------------------------

/** Describes one field in a TOML config section. */
export interface FieldDef {
  /** Key in the flat form-data object (e.g. "pruning_enabled"). */
  formKey: string;
  /** Key in the TOML file (e.g. "enabled"). */
  tomlKey: string;
  /**
   * Dot-separated path to the TOML section header.
   * Examples: "pruning", "pruning.progressive", "docker.default_env".
   * Omit (or set to "") for root-level fields.
   */
  section?: string;
  /** Value type for serialization. */
  type: 'string' | 'number' | 'boolean' | 'string[]' | 'record' | 'table[]';
  /** Default value used during deserialization when the field is missing. */
  defaultValue: unknown;
  /**
   * If true, omit this field from TOML output when the value
   * equals defaultValue, or is null / empty-string / empty-array.
   */
  optional?: boolean;
  /**
   * For 'table[]' type only: sub-schema describing each table entry.
   * Produces `[[section.tomlKey]]` blocks.
   */
  subSchema?: FieldDef[];
}

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

/** Escape a string for embedding in a TOML double-quoted value.
 *  Backslashes and double-quotes must be escaped so that paths like
 *  `C:\Program Files\...` produce valid TOML. */
export function escapeTomlString(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

// ---------------------------------------------------------------------------
// Serialization  (form data -> TOML string)
// ---------------------------------------------------------------------------

/** Check whether a value is "empty" for the purpose of optional omission. */
function isEmpty(value: unknown, type: FieldDef['type']): boolean {
  if (value === null || value === undefined) return true;
  if (type === 'string' && value === '') return true;
  if (type === 'string[]' && Array.isArray(value) && value.length === 0) return true;
  if (type === 'record' && typeof value === 'object' && Object.keys(value as object).length === 0) return true;
  if (type === 'table[]' && Array.isArray(value) && value.length === 0) return true;
  return false;
}

/**
 * Serialize a flat form-data object to a TOML string using the given schema.
 *
 * Fields are grouped by their `section` and emitted in schema order.
 * Section headers (`[section]`) are emitted once, before the first field
 * belonging to that section.
 */
export function serializeToml(
  data: Record<string, unknown>,
  schema: FieldDef[],
): string {
  const lines: string[] = [];
  let currentSection = '';

  for (const field of schema) {
    const section = field.section ?? '';
    const value = data[field.formKey];

    // Handle 'record' type as a dedicated TOML section with key-value pairs.
    if (field.type === 'record') {
      const rec = (value ?? {}) as Record<string, string>;
      if (field.optional && Object.keys(rec).length === 0) continue;
      lines.push('');
      const sectionPath = section ? `${section}.${field.tomlKey}` : field.tomlKey;
      lines.push(`[${sectionPath}]`);
      for (const [k, v] of Object.entries(rec)) {
        lines.push(`${k} = "${escapeTomlString(v)}"`);
      }
      currentSection = sectionPath;
      continue;
    }

    // Handle 'table[]' type as TOML array-of-tables.
    if (field.type === 'table[]') {
      const arr = (value ?? []) as Record<string, unknown>[];
      if (field.optional && arr.length === 0) continue;
      const tablePath = section ? `${section}.${field.tomlKey}` : field.tomlKey;
      for (const entry of arr) {
        lines.push('');
        lines.push(`[[${tablePath}]]`);
        if (field.subSchema) {
          for (const sub of field.subSchema) {
            const subVal = entry[sub.formKey] ?? entry[sub.tomlKey];
            if (sub.optional && isEmpty(subVal, sub.type)) continue;
            lines.push(formatValue(sub.tomlKey, subVal, sub.type));
          }
        }
      }
      currentSection = tablePath;
      continue;
    }

    // Skip optional fields whose value is empty or matches the default.
    if (field.optional && isEmpty(value, field.type)) continue;
    if (field.optional && value === field.defaultValue) continue;

    // Emit section header if we've moved to a new section.
    if (section !== currentSection) {
      if (section) {
        lines.push('');
        lines.push(`[${section}]`);
      }
      currentSection = section;
    }

    lines.push(formatValue(field.tomlKey, value, field.type));
  }

  return lines.join('\n') + '\n';
}

/** Format a single `key = value` TOML line. */
function formatValue(key: string, value: unknown, type: FieldDef['type']): string {
  switch (type) {
    case 'string':
      return `${key} = "${escapeTomlString(String(value ?? ''))}"`;
    case 'number':
      return `${key} = ${value}`;
    case 'boolean':
      return `${key} = ${value}`;
    case 'string[]': {
      const arr = (value as string[]) ?? [];
      return `${key} = [${arr.map((s) => `"${escapeTomlString(s)}"`).join(', ')}]`;
    }
    default:
      return `${key} = "${escapeTomlString(String(value ?? ''))}"`;
  }
}

// ---------------------------------------------------------------------------
// Deserialization  (parsed JSON -> flat form data)
// ---------------------------------------------------------------------------

/**
 * Walk a dot-separated path into a nested JSON object.
 * e.g. resolve(json, "pruning.progressive") -> json?.pruning?.progressive
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function resolve(obj: any, path: string): any {
  if (!path) return obj;
  return path.split('.').reduce((cur, key) => cur?.[key], obj);
}

/**
 * Deserialize a parsed JSON config (from `config_get`) into a flat
 * form-data record, applying defaults from the schema.
 *
 * @param json - The parsed JSON object for a single config section
 *               (e.g. `allConfig.session`).
 * @param schema - The field schema for that section.
 * @param postProcess - Optional callback for backward-compat transforms
 *                      that cannot be expressed declaratively.
 */
export function deserializeFromJson(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  json: any,
  schema: FieldDef[],
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  postProcess?: (result: Record<string, any>, json: any) => void,
): Record<string, unknown> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const result: Record<string, any> = {};

  for (const field of schema) {
    const section = field.section ?? '';
    const parent = resolve(json, section);

    if (field.type === 'table[]') {
      const arr = parent?.[field.tomlKey] ?? [];
      if (!Array.isArray(arr)) {
        result[field.formKey] = field.defaultValue;
        continue;
      }
      if (field.subSchema) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        result[field.formKey] = arr.map((entry: any) => {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const row: Record<string, any> = {};
          for (const sub of field.subSchema!) {
            row[sub.formKey] = entry?.[sub.tomlKey] ?? sub.defaultValue;
          }
          return row;
        });
      } else {
        result[field.formKey] = arr;
      }
      continue;
    }

    if (field.type === 'record') {
      // Records are stored as a sub-object keyed by tomlKey inside the section.
      const rec = parent?.[field.tomlKey];
      result[field.formKey] = (rec && typeof rec === 'object') ? rec : field.defaultValue;
      continue;
    }

    const raw = parent?.[field.tomlKey];

    if (field.type === 'string[]') {
      result[field.formKey] = Array.isArray(raw) ? raw : field.defaultValue;
    } else {
      result[field.formKey] = raw ?? field.defaultValue;
    }
  }

  if (postProcess) {
    postProcess(result, json);
  }

  return result;
}

// ---------------------------------------------------------------------------
// Comment-preserving merge  (raw TOML + form data -> patched TOML)
// ---------------------------------------------------------------------------

/**
 * Merge updated form-data values back into the original raw TOML text,
 * preserving comments, blank lines, and unrecognized fields.
 *
 * Algorithm:
 *   1. Parse lines to track the current `[section]` context.
 *   2. For each `key = value` line whose key+section matches a schema field,
 *      replace the value portion with the new value from `data`.
 *   3. Schema fields not found in the raw text are appended at the end of
 *      their respective section.
 *   4. Everything else (comments, blank lines, unknown keys) passes through.
 *
 * Falls back to `serializeToml` when `rawToml` is empty or undefined.
 */
export function mergeIntoRawToml(
  rawToml: string | undefined,
  data: Record<string, unknown>,
  schema: FieldDef[],
): string {
  if (!rawToml || rawToml.trim() === '') {
    return serializeToml(data, schema);
  }

  // Build lookup: "section::tomlKey" -> FieldDef
  const fieldMap = new Map<string, FieldDef>();
  for (const f of schema) {
    // Skip record and table[] types -- they need special handling.
    if (f.type === 'record' || f.type === 'table[]') continue;
    const key = `${f.section ?? ''}::${f.tomlKey}`;
    fieldMap.set(key, f);
  }

  // Track which schema fields have been written (by formKey).
  const written = new Set<string>();

  const lines = rawToml.split('\n');
  const result: string[] = [];
  let currentSection = '';

  // Regex for section headers: [section] or [[section]]
  const sectionRe = /^\s*\[{1,2}([^\]]+)\]{1,2}\s*$/;
  // Regex for key = value lines (captures key and the rest after =).
  const kvRe = /^(\s*)([\w][\w.-]*)\s*=\s*(.*)/;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // Skip blank lines and comments -- pass through.
    if (trimmed === '' || trimmed.startsWith('#')) {
      result.push(line);
      continue;
    }

    // Section header.
    const sectionMatch = trimmed.match(sectionRe);
    if (sectionMatch) {
      currentSection = sectionMatch[1].trim();
      result.push(line);
      continue;
    }

    // Key = value line.
    const kvMatch = line.match(kvRe);
    if (kvMatch) {
      const indent = kvMatch[1];
      const tomlKey = kvMatch[2];
      const lookupKey = `${currentSection}::${tomlKey}`;
      const field = fieldMap.get(lookupKey);

      if (field) {
        // Replace this line with the updated value.
        const value = data[field.formKey];

        // If field is optional and value is empty, keep the line commented out.
        if (field.optional && isEmpty(value, field.type)) {
          // Keep original line (might be a user-commented-out line, or write nothing).
          result.push(line);
        } else {
          result.push(`${indent}${formatValue(tomlKey, value, field.type)}`);
        }
        written.add(field.formKey);
      } else {
        // Not a schema-managed field -- pass through unchanged.
        result.push(line);
      }
      continue;
    }

    // Anything else (e.g. multi-line values) -- pass through.
    result.push(line);
  }

  // Append schema fields that were not found in the raw TOML.
  // Group by section so we can insert them under the right header.
  const missingBySection = new Map<string, FieldDef[]>();
  for (const field of schema) {
    if (field.type === 'record' || field.type === 'table[]') continue;
    if (written.has(field.formKey)) continue;

    const value = data[field.formKey];
    // Don't append optional fields with empty values.
    if (field.optional && isEmpty(value, field.type)) continue;
    if (field.optional && value === field.defaultValue) continue;

    const section = field.section ?? '';
    if (!missingBySection.has(section)) {
      missingBySection.set(section, []);
    }
    missingBySection.get(section)!.push(field);
  }

  // Check which sections already exist in the output.
  const existingSections = new Set<string>();
  existingSections.add(''); // Root always "exists".
  for (const line of result) {
    const m = line.trim().match(sectionRe);
    if (m) existingSections.add(m[1].trim());
  }

  for (const [section, fields] of missingBySection) {
    if (!existingSections.has(section)) {
      // Need to add the section header.
      result.push('');
      result.push(`[${section}]`);
      existingSections.add(section);
    }

    for (const field of fields) {
      const value = data[field.formKey];
      result.push(formatValue(field.tomlKey, value, field.type));
    }
  }

  // Handle record and table[] types -- these require special merge logic.
  // For simplicity, we handle them by:
  // 1. Records: find existing [section.key] block and replace values,
  //    or append the block if missing.
  // 2. table[]: fully regenerated (these are dynamic-length arrays,
  //    comment preservation is not practical).

  for (const field of schema) {
    if (field.type !== 'record' && field.type !== 'table[]') continue;

    const value = data[field.formKey];
    const section = field.section ?? '';

    if (field.type === 'record') {
      const rec = (value ?? {}) as Record<string, string>;
      if (field.optional && Object.keys(rec).length === 0) continue;
      const sectionPath = section ? `${section}.${field.tomlKey}` : field.tomlKey;

      // Check if this record section already exists and remove it.
      const sectionHeader = `[${sectionPath}]`;
      const headerIdx = result.findIndex(
        (l) => l.trim() === sectionHeader,
      );
      if (headerIdx >= 0) {
        // Remove header and all following key=value lines until next section or blank.
        let endIdx = headerIdx + 1;
        while (endIdx < result.length) {
          const t = result[endIdx].trim();
          if (t === '' || t.startsWith('[') || t.startsWith('#')) break;
          endIdx++;
        }
        // Replace with new content.
        const newLines = [sectionHeader];
        for (const [k, v] of Object.entries(rec)) {
          newLines.push(`${k} = "${escapeTomlString(v)}"`);
        }
        result.splice(headerIdx, endIdx - headerIdx, ...newLines);
      } else if (Object.keys(rec).length > 0) {
        result.push('');
        result.push(sectionHeader);
        for (const [k, v] of Object.entries(rec)) {
          result.push(`${k} = "${escapeTomlString(v)}"`);
        }
      }
    }

    if (field.type === 'table[]') {
      const arr = (value ?? []) as Record<string, unknown>[];
      const tablePath = section ? `${section}.${field.tomlKey}` : field.tomlKey;
      const arrayHeader = `[[${tablePath}]]`;

      // Remove all existing [[tablePath]] blocks.
      let idx = 0;
      while (idx < result.length) {
        if (result[idx].trim() === arrayHeader) {
          // Remove this block: header + following kv lines.
          let endIdx = idx + 1;
          while (endIdx < result.length) {
            const t = result[endIdx].trim();
            if (t === '' || t.startsWith('[')) break;
            endIdx++;
          }
          // Also remove a preceding blank line if present.
          const removeStart = idx > 0 && result[idx - 1].trim() === '' ? idx - 1 : idx;
          result.splice(removeStart, endIdx - removeStart);
          // Don't advance idx -- the splice shifted everything down.
        } else {
          idx++;
        }
      }

      // Append new entries.
      if (!(field.optional && arr.length === 0)) {
        for (const entry of arr) {
          result.push('');
          result.push(arrayHeader);
          if (field.subSchema) {
            for (const sub of field.subSchema) {
              const subVal = entry[sub.formKey] ?? entry[sub.tomlKey];
              if (sub.optional && isEmpty(subVal, sub.type)) continue;
              result.push(formatValue(sub.tomlKey, subVal, sub.type));
            }
          }
        }
      }
    }
  }

  // Ensure trailing newline.
  const joined = result.join('\n');
  return joined.endsWith('\n') ? joined : joined + '\n';
}
