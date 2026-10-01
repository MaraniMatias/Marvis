import { describe, expect, it } from "vitest";
import { terminalPathIn, terminalPathsIn } from "./terminal-paths";

/** The characters a path covers, which is what an underline has to land on. */
function at(line: string, path: string) {
  const start = line.indexOf(path);
  return { start, end: start + path.length };
}

describe("terminalPathIn", () => {
  it("takes a path out of a token and drops what framed it", () => {
    expect(terminalPathIn("src/lib/marvis-terminal.ts:")).toBe("src/lib/marvis-terminal.ts");
    expect(terminalPathIn("(src/App.vue)")).toBe("src/App.vue");
    expect(terminalPathIn("`Cargo.toml`")).toBe("Cargo.toml");
    expect(terminalPathIn('"my notes.md"')).toBe("my notes.md");
  });

  it("leaves off the line and column a compiler appended", () => {
    expect(terminalPathIn("src/main.rs:42:10")).toBe("src/main.rs");
    expect(terminalPathIn("src/main.rs:42")).toBe("src/main.rs");
    // A colon that is not a position is part of the name and stays.
    expect(terminalPathIn("weird:name/file.ts")).toBe("weird:name/file.ts");
  });

  it("refuses what is not a path", () => {
    expect(terminalPathIn("error:")).toBeNull();
    expect(terminalPathIn("error: something broke")).toBeNull();
    expect(terminalPathIn("...")).toBeNull();
    expect(terminalPathIn("..")).toBeNull();
    expect(terminalPathIn("--verbose")).toBeNull();
    // A URL is a link, not a file: nothing on this disk is named that.
    expect(terminalPathIn("https://example.com/a.ts")).toBeNull();
    // A word with no separator and no extension is indistinguishable from prose.
    expect(terminalPathIn("error")).toBeNull();
    expect(terminalPathIn("Makefile")).toBeNull();
  });
});

describe("terminalPathsIn", () => {
  it("reads every path off a line, in the order printed", () => {
    const line = "error[E0308]: mismatched types → src/lib/foo.rs:12:5 and src/main.rs";
    expect(terminalPathsIn(line)).toEqual([
      { path: "src/lib/foo.rs", ...at(line, "src/lib/foo.rs") },
      { path: "src/main.rs", ...at(line, "src/main.rs") },
    ]);
  });

  it("does not find a quoted path twice", () => {
    const line = 'could not read "src/lib/foo.ts"';
    expect(terminalPathsIn(line)).toEqual([{ path: "src/lib/foo.ts", ...at(line, "src/lib/foo.ts") }]);
  });

  it("keeps the underline off the quote and the line number", () => {
    const line = '  in "src/App.vue:12:4", and again in src/App.vue';
    const [first, second] = terminalPathsIn(line);
    // The quoted one stops at the file, not at the `:12:4` that framed it.
    expect(first).toEqual({ path: "src/App.vue", start: 6, end: 17 });
    expect(second).toEqual({ path: "src/App.vue", start: 38, end: 49 });
  });

  it("keeps the ./ a shell prints, because that is the file it means", () => {
    expect(terminalPathsIn("writing ./src/App.vue")).toEqual([{ path: "./src/App.vue", start: 8, end: 21 }]);
  });

  it("finds nothing on a line that mentions no file", () => {
    expect(terminalPathsIn("Compiling 3 files, nothing to report")).toEqual([]);
    expect(terminalPathsIn("")).toEqual([]);
  });
});
