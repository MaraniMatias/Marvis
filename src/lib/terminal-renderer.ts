export interface TerminalRenderer {
  write(data: Uint8Array, callback?: () => void): void;
}

export function renderPtyOutput(renderer: TerminalRenderer, buffer: ArrayBuffer, callback?: () => void): void {
  renderer.write(new Uint8Array(buffer), callback);
}
