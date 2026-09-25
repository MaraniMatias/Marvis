export interface EditorPosition {
  line: number;
  column: number;
}

export function parseEditorPosition(value: string): EditorPosition | null {
  const match = /^([1-9]\d*)(?::([1-9]\d*))?$/.exec(value.trim());
  if (!match) return null;
  const line = Number(match[1]);
  const column = Number(match[2] ?? 1);
  if (!Number.isSafeInteger(line) || !Number.isSafeInteger(column) || line > 0xffff_ffff || column > 0xffff_ffff) {
    return null;
  }
  return { line, column };
}
