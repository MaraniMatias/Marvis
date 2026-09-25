import { describe, expect, it } from "vitest";
import { parseEditorPosition } from "./editor";

describe("editor locations", () => {
  it("parses line and optional column values as 1-based positions", () => {
    expect(parseEditorPosition("42")).toEqual({ line: 42, column: 1 });
    expect(parseEditorPosition(" 42:7 ")).toEqual({ line: 42, column: 7 });
  });

  it("rejects invalid or out-of-range positions", () => {
    expect(parseEditorPosition("0")).toBeNull();
    expect(parseEditorPosition("4:0")).toBeNull();
    expect(parseEditorPosition("4:5:6")).toBeNull();
    expect(parseEditorPosition("4294967296")).toBeNull();
  });
});
