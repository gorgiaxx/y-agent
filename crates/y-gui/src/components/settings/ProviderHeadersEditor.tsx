import { X } from 'lucide-react';

import { Button } from '../ui/Button';
import { Input } from '../ui/Input';

export function ProviderHeadersEditor({
  headers,
  onChange,
}: {
  headers: Record<string, string>;
  onChange: (headers: Record<string, string>) => void;
}) {
  return (
    <div className="pf-kv-list">
      {Object.entries(headers).map(([key, value], index) => (
        <div key={index} className="pf-kv-row">
          <Input
            style={{ flex: 1 }}
            value={key}
            onChange={(event) => {
              const entries = Object.entries(headers);
              entries[index] = [event.target.value, value];
              onChange(Object.fromEntries(entries));
            }}
            placeholder="Header"
          />
          <span className="pf-kv-sep">:</span>
          <Input
            style={{ flex: 2 }}
            value={value}
            onChange={(event) => {
              onChange({ ...headers, [key]: event.target.value });
            }}
            placeholder="value"
          />
          <Button
            variant="icon"
            size="sm"
            title="Remove"
            onClick={() => {
              const next = { ...headers };
              delete next[key];
              onChange(next);
            }}
          >
            <X size={12} />
          </Button>
        </div>
      ))}
      <Button
        variant="ghost"
        size="sm"
        className="pf-kv-add"
        onClick={() => onChange({ ...headers, '': '' })}
      >
        + Add Header
      </Button>
    </div>
  );
}
