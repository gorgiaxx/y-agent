export interface IdeInfo {
  id: string;
  name: string;
  command: string;
  available: boolean;
}

export function visibleIdeOptions(options: IdeInfo[]): IdeInfo[] {
  return options.filter((option) => option.id === 'auto' || option.available);
}
