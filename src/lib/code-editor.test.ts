// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { defaultHighlightStyle, forceParsing, syntaxTree } from "@codemirror/language";
import { redo, undo } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";
import { createCodeEditor } from "./code-editor";

const views = new Set<EditorView>();
const hosts = new Set<HTMLElement>();
let initialStyles = new Set<HTMLStyleElement>();

beforeEach(() => {
  initialStyles = new Set(document.querySelectorAll("style"));
});

afterEach(() => {
  for (const view of views) view.destroy();
  views.clear();
  for (const host of hosts) host.remove();
  hosts.clear();
  for (const style of document.querySelectorAll("style")) {
    if (!initialStyles.has(style)) style.remove();
  }
});

function mount(language: string, content: string): EditorView {
  const host = document.createElement("div");
  hosts.add(host);
  document.body.appendChild(host);
  const view = createCodeEditor({
    parent: host,
    content,
    language,
    readingPosition: { top: 0, left: 0 },
    onChange: () => {},
    onScroll: () => {},
  });
  views.add(view);
  return view;
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
function colorForSpan(span: Element): string | undefined {
  const classes = span.className.split(/\s+/);
  if (classes.length === 0) return undefined;
  const css = Array.from(document.querySelectorAll("style"))
    .map((style) => style.textContent ?? "")
    .join("}");
  for (const block of css.split("}")) {
    const rule = block.match(/^\s*\.(.+?)\s*\{\s*(?:color:\s*(#[0-9a-f]{3,8}))?/i);
    if (rule?.[2] && rule[1].split(/\s+/).some((name) => classes.includes(name))) return rule[2];
  }
  return undefined;
}

function colorPaintedOn(view: EditorView, text: string): string | undefined {
  const span = Array.from(view.contentDOM.querySelectorAll("span")).find((node) => node.textContent === text);
  return span ? colorForSpan(span) : undefined;
}

/** Every node name in the tree, so an assertion can name a construct instead of a position. */
function nodeNames(view: EditorView): string[] {
  const names: string[] = [];
  syntaxTree(view.state).iterate({
    enter: (node) => {
      names.push(node.name);
    },
  });
  return names;
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
  });

  it("does not read an escaped hash as a comment, since git only honours one at the line start", () => {
    const view = mount("gitignore", "\\#not-a-comment");
    expect(tokensByLine(view).get("\\#not-a-comment")).toBeUndefined();
  });

  it("reads the metadata a Markdown document opens with as the YAML it is", () => {
    const view = mount(
      "markdown",
      ["---", "name: tamis", 'color: "#F2B84B"', "# a note", "---", "", "# Tamis", "", "Body."].join("\n"),
    );
    expect(forceParsing(view, view.state.doc.length)).toBe(true);

    // A key and its value are YAML, so they take the colors YAML gives them rather than the flat
    // text the Markdown grammar would leave the block as. The comment is YAML's too.
    expect(colorPaintedOn(view, "name")).toBe("#79c0ff");
    expect(colorPaintedOn(view, "# a note")).toBe("#8b949e");

    // The document behind the block is still read as Markdown: a heading is still a heading.
    expect(nodeNames(view)).toContain("ATXHeading1");
  });

  it("leaves a document that only opens with a rule alone", () => {
    // A rule with nothing to close it is not metadata, so nothing in it is read as YAML: the words
    // stay the flat text the Markdown grammar leaves them, and the rule is a rule.
    const view = mount("markdown", "---\nname: thing\n\n# Heading");
    expect(forceParsing(view, view.state.doc.length)).toBe(true);

    expect(colorPaintedOn(view, "name")).toBeUndefined();
    expect(nodeNames(view)).toContain("HorizontalRule");
    expect(nodeNames(view)).toContain("ATXHeading1");
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
  });

  it("paints no token in the light palette, which is the one it no longer carries", () => {
    // Nothing injects `defaultHighlightStyle` as a fallback any more, so this list is the only
    // palette in play. A token still painted from the light one is a dark red keyword on `#17191f`,
    // and it would only show up in a language nobody happened to open while this was written.
    const light = new Set(
      defaultHighlightStyle.specs.flatMap((spec) => (spec.color ? [spec.color.toLowerCase()] : [])),
    );
    const documents = [
      ["typescript", "const answer: number = 42;\n// note"],
      ["python", "def answer():\n    return 42"],
      ["json", '{"answer": 42, "nested": {"deep": [true, null]}}'],
      ["yaml", "answer: 42\nnested:\n  deep: true"],
      ["markdown", ["---", "name: tamis", "---", "", "# Title", "", "- item", "> quote"].join("\n")],
      ["gitignore", "# deps\nnode_modules\n!.env.example\n*.log"],
    ] as const;

    for (const [language, content] of documents) {
      const view = mount(language, content);
      const fromLightPalette = Array.from(view.contentDOM.querySelectorAll("span"))
        .map((span) => colorForSpan(span))
        .filter((color): color is string => color !== undefined && light.has(color.toLowerCase()));

      expect({ language, fromLightPalette }).toEqual({ language, fromLightPalette: [] });
    }
  });

  it("has the gutter and the undo history the extension list claims", () => {
    const view = mount("typescript", "const answer = 1;");

    expect(view.dom.querySelector(".cm-lineNumbers")).not.toBeNull();

    // The history is one line of the list nothing else pulls in, so a typo there is invisible
    // until someone types into the document and then cannot take it back.
    view.dispatch({ changes: { from: view.state.doc.length, insert: "// typed" } });
    expect(view.state.doc.toString()).toBe("const answer = 1;// typed");
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe("const answer = 1;");
    expect(redo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe("const answer = 1;// typed");
  });
});
