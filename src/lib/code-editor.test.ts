// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { defaultHighlightStyle, syntaxTree } from "@codemirror/language";
import type { EditorView } from "@codemirror/view";
import { createCodeEditor, marvisHighlightStyle } from "./code-editor";

function mount(language: string, content: string): EditorView {
  return createCodeEditor({
    parent: document.createElement("div"),
    content,
    language,
    readingPosition: { top: 0, left: 0 },
    onChange: () => {},
    onScroll: () => {},
  });
}

/**
 * The token the highlighter produced for each line, keyed by the text it covers, so an assertion
 * can name the piece of a gitignore it is about rather than a position that shifts with the file.
 */
function tokensByLine(view: EditorView): Map<string, string[]> {
  const tokens = new Map<string, string[]>();
  syntaxTree(view.state).iterate({
    enter: (node) => {
      if (node.name === "Document") return;
      const text = view.state.sliceDoc(node.from, node.to);
      const named = tokens.get(text);
      if (named) named.push(node.name);
      else tokens.set(text, [node.name]);
    },
  });
  return tokens;
}

/** The color the editor actually painted a token with, read back out of the injected stylesheet. */
function colorPaintedOn(view: EditorView, text: string): string | undefined {
  const span = Array.from(view.contentDOM.querySelectorAll("span")).find((node) => node.textContent === text);
  if (!span) return undefined;
  const classes = span.className.split(/\s+/);
  const css = Array.from(document.querySelectorAll("style"))
    .map((style) => style.textContent ?? "")
    .join("}");
  for (const block of css.split("}")) {
    const rule = block.match(/^\s*\.(.+?)\s*\{\s*(?:color:\s*(#[0-9a-f]{3,8}))?/i);
    if (rule?.[2] && rule[1].split(/\s+/).some((name) => classes.includes(name))) return rule[2];
  }
  return undefined;
}

describe("code editor", () => {
  it("reads a gitignore as comments, negations and globs, never as one long key", () => {
    const view = mount("gitignore", "# deps\nnode_modules\n!.env.example\n*.log");
    const tokens = tokensByLine(view);

    expect(tokens.get("# deps")).toEqual(["comment"]);
    // The `properties` mode that serves `.gitconfig` reads a bare glob as one long `def` run and
    // paints a leading `!` as a comment, which is the opposite of what a negation means.
    expect(tokens.get("node_modules")).toBeUndefined();
    expect(tokens.get("!")).toEqual(["keyword"]);
    expect(tokens.get("*")).toEqual(["operator"]);
    expect([...tokens.values()].flat().some((name) => name?.includes("definition"))).toBe(false);

    view.destroy();
  });

  it("does not read an escaped hash as a comment, since git only honours one at the line start", () => {
    const view = mount("gitignore", "\\#not-a-comment");
    expect(tokensByLine(view).get("\\#not-a-comment")).toBeUndefined();

    view.destroy();
  });

  it("paints tokens in the dark palette the read-only preview already uses", () => {
    const view = mount("typescript", "const answer: number = 42;\n// note");

    // `defaultHighlightStyle` is a light palette — dark red keywords, mid-blue strings — that was
    // laid over Marvis' `#17191f` editor background, where the darker half is barely readable.
    expect(colorPaintedOn(view, "const")).toBe("#ff7b72");
    expect(colorPaintedOn(view, "answer")).toBe("#ffa657");
    expect(colorPaintedOn(view, "number")).toBe("#7ee787");
    expect(colorPaintedOn(view, "42")).toBe("#79c0ff");
    expect(colorPaintedOn(view, "// note")).toBe("#8b949e");

    view.destroy();
  });

  it("leaves no tag of the light palette to show through", () => {
    // `basicSetup` goes on injecting `defaultHighlightStyle`, so this list is only as good as the
    // tags it happens to cover. A tag the light palette colors and this one does not is a token
    // painted `#30a` on a `#17191f` background, and it would only show up in a language nobody
    // happened to open while this was written.
    const coloredBy = (style: typeof marvisHighlightStyle) =>
      new Set(
        style.specs
          .flatMap((spec) => (Array.isArray(spec.tag) ? spec.tag : [spec.tag]))
          .filter((tag) => tag)
          .map((tag) => tag.id),
      );
    const dark = coloredBy(marvisHighlightStyle);
    const lightThrough = defaultHighlightStyle.specs.flatMap((spec) =>
      (Array.isArray(spec.tag) ? spec.tag : [spec.tag])
        .filter((tag) => spec.color && tag && !dark.has(tag.id))
        .map((tag) => `${tag.label ?? tag.id} as ${spec.color}`),
    );

    expect(lightThrough).toEqual([]);
  });
});
