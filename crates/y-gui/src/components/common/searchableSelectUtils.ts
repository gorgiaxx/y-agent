export interface SearchableOption {
  value: string;
  label: string;
  description?: string;
  keywords?: string[];
}

export function filterSearchableOptions(
  options: SearchableOption[],
  query: string,
): SearchableOption[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (!normalizedQuery) return options;

  return options.filter((option) => [
    option.label,
    option.description ?? '',
    ...(option.keywords ?? []),
  ].some((value) => value.toLocaleLowerCase().includes(normalizedQuery)));
}
