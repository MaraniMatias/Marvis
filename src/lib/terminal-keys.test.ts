import { describe, expect, it, vi } from "vitest";
import { watchKeyboardProtocol } from "./terminal-keys";

/** The escape every one of these sequences opens with, as the code it is. */
const ESC = String.fromCharCode(0x1b);

function output(text: string): ArrayBuffer {
  const encoded = new TextEncoder().encode(text);
  return encoded.buffer.slice(encoded.byteOffset, encoded.byteOffset + encoded.byteLength) as ArrayBuffer;
}

describe("watchKeyboardProtocol", () => {
  it("leaves a program that never announced anything to be sent the line ending", () => {
    const answer = vi.fn();
    const protocol = watchKeyboardProtocol(answer);
    protocol.read(output(`${ESC}[?2026h${ESC}[38;5;196mhello`));
    expect(protocol.csiU).toBe(false);
    expect(answer).not.toHaveBeenCalled();
  });

  it("takes the encoding from the flags a program pushed, including the flag that turns it off", () => {
    const protocol = watchKeyboardProtocol(vi.fn());
    protocol.read(output(`hello${ESC}[>0u`));
    expect(protocol.csiU).toBe(true);
    // Still on with the flags that report escape codes and event types alongside.
    protocol.read(output(`${ESC}[>14u`));
    expect(protocol.csiU).toBe(true);
    // The older form says the same thing.
    protocol.read(output(`${ESC}[=0;2u`));
    expect(protocol.csiU).toBe(true);
    // One bit is all it takes to turn it off.
    protocol.read(output(`${ESC}[>1u`));
    expect(protocol.csiU).toBe(false);
  });

  it("answers a query with what this terminal supports without taking that as the program agreeing", () => {
    const answer = vi.fn();
    const protocol = watchKeyboardProtocol(answer);
    protocol.read(output(`${ESC}[?u`));
    expect(answer).toHaveBeenCalledWith(`${ESC}[?0u`);
    expect(protocol.csiU).toBe(false);
    // The push that follows the answer is what turns it on.
    protocol.read(output(`${ESC}[>0u`));
    expect(protocol.csiU).toBe(true);
  });

  it("reads an announcement the channel boundary cut in half", () => {
    const protocol = watchKeyboardProtocol(vi.fn());
    protocol.read(output(`${ESC}[>`));
    expect(protocol.csiU).toBe(false);
    protocol.read(output("0u"));
    expect(protocol.csiU).toBe(true);
  });

  it("leaves its own answer, echoed back, as nothing to obey", () => {
    const protocol = watchKeyboardProtocol(vi.fn());
    // A shell with echo on hands the answer straight back, and a question with digits behind it is
    // not a question.
    protocol.read(output(`${ESC}[?0u`));
    expect(protocol.csiU).toBe(false);
  });
});
