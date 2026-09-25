import { Terminal } from "@xterm/headless";
import { describe, expect, it } from "vitest";
import { renderPtyOutput } from "./terminal-renderer";

function write(terminal: Terminal, bytes: Uint8Array) {
  return new Promise<void>((resolve) => {
    const buffer = bytes.slice().buffer as ArrayBuffer;
    renderPtyOutput(terminal, buffer, resolve);
  });
}

describe("PTY output renderer adapter", () => {
  it("passes split UTF-8 and ANSI byte chunks intact to xterm", async () => {
    const terminal = new Terminal({ allowProposedApi: true, cols: 40, rows: 5 });
    const output = new TextEncoder().encode("\u001b[31mred λ\u001b[0m\r\nsecond line\r\n");

    await write(terminal, output.subarray(0, 5));
    await write(terminal, output.subarray(5, 8));
    await write(terminal, output.subarray(8));

    expect(terminal.buffer.active.getLine(0)?.translateToString(true)).toBe("red λ");
    expect(terminal.buffer.active.getLine(1)?.translateToString(true)).toBe("second line");
    expect(terminal.buffer.active.getLine(0)?.getCell(0)?.getFgColor()).toBe(1);
    terminal.dispose();
  });

  it("preserves alternate-screen transitions through the same byte adapter", async () => {
    const terminal = new Terminal({ allowProposedApi: true, cols: 40, rows: 5 });
    await write(terminal, new TextEncoder().encode("main text\r\n"));
    await write(terminal, new TextEncoder().encode("\u001b[?1049hALT screen"));

    expect(terminal.buffer.active.type).toBe("alternate");
    expect(
      Array.from({ length: terminal.rows }, (_, row) =>
        terminal.buffer.active.getLine(row)?.translateToString(true),
      ).join(" "),
    ).toContain("ALT screen");

    await write(terminal, new TextEncoder().encode("\u001b[?1049l"));
    expect(terminal.buffer.active.type).toBe("normal");
    expect(terminal.buffer.active.getLine(0)?.translateToString(true)).toBe("main text");
    terminal.dispose();
  });
});
