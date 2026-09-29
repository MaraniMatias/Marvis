import { describe, expect, it } from "vitest";
import { absoluteFilePath } from "./files";

describe("absoluteFilePath", () => {
  it("joins the checkout directory and the file's own path with one separator", () => {
    expect(absoluteFilePath("/Users/dev/marvis", "src/app.ts")).toBe("/Users/dev/marvis/src/app.ts");
  });

  it("does not double the separator when the checkout directory carries a trailing one", () => {
    expect(absoluteFilePath("/Users/dev/marvis/", "docs/readme.md")).toBe("/Users/dev/marvis/docs/readme.md");
    expect(absoluteFilePath("/Users/dev/marvis///", "docs/readme.md")).toBe("/Users/dev/marvis/docs/readme.md");
  });

  it("leaves a checkout directory that is only separators to the file's own path", () => {
    expect(absoluteFilePath("/", "notes.txt")).toBe("/notes.txt");
  });

  it("uses the review root for exported files", () => {
    expect(absoluteFilePath("/repo", "notes.md", "review", "/Users/dev/.marvis/tmp/code-reviews/")).toBe(
      "/Users/dev/.marvis/tmp/code-reviews/notes.md",
    );
  });
});
