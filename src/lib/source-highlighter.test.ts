// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import {
  highlightCodeBlock,
  highlightSource,
  highlightSourceAs,
  languageForFenceInfo,
  sanitizeHighlightedHtml,
} from "./source-highlighter";
import { PLAIN_TEXT } from "./source-languages";

describe("source highlighter", () => {
  it("renders in the stylesheet's tokens rather than in a palette of its own", async () => {
    // One render, resolved by the browser against whichever palette is in effect. A hex here is a
    // palette of its own: it is what would stop a file from repainting when the theme changes.
    const lines = await highlightSource("src/tokens.ts", "const answer: number = 42;\n// note");

    expect(lines?.join("")).toContain("var(--marvis-syntax-token-keyword)");
    expect(lines?.join("")).not.toMatch(/#[0-9a-f]{6}\b/i);
  });

  it("highlights known source and reuses the path/content cache entry", async () => {
    const first = highlightSource("src/cache.ts", "const answer: number = 42;\nreturn answer;");
    const second = highlightSource("src/cache.ts", "const answer: number = 42;\nreturn answer;");

    expect(second).toBe(first);
    const lines = await first;
    expect(lines).toHaveLength(2);
    expect(lines?.[0]).toContain('class="line"');
    expect(lines?.[0]).toContain("color:");
  });

  it("reads the same source as whichever grammar it is told to, and caches one per grammar", async () => {
    const source = "answer = 42";
    const python = highlightSourceAs("python", source);
    expect(highlightSourceAs("python", source)).toBe(python);
    const lines = await python;
    expect(lines?.[0]).toContain('class="line"');

    // The same text under another grammar is another render, which is the whole point of forcing
    // one: a cache keyed by anything but the grammar would hand back the first render.
    const shell = highlightSourceAs("shellscript", source);
    expect(shell).not.toBe(python);
    expect(await shell).not.toEqual(lines);
  });

  it("reads a grammar the reader chose over one the extension never gave", async () => {
    // This is what the toolbar's forced grammar reaches: a `.txt` is plain on its own.
    expect(await highlightSource("notes.txt", "answer = 42")).toBeNull();
    expect(await highlightSourceAs("python", "answer = 42")).toHaveLength(1);
  });

  it("colors the config dotfiles that carry no extension, and keeps a negation out of a comment", async () => {
    // `gitignore` has no grammar of its own in Shiki, so it renders as `ini` under a separate name.
    // If the indirection ever drops that name, this is the file that goes back to plain.
    const lines = await highlightSource(".gitignore", "# deps\nnode_modules\n!.env.example");
    expect(lines).toHaveLength(3);
    expect(lines?.[0]).toContain("var(--marvis-syntax-token-comment)");
    // `!.env.example` re-includes a pattern, so it must not be painted as the `#` comment above.
    expect(lines?.[2]).not.toContain("var(--marvis-syntax-token-comment)");

    const env = await highlightSource(".env.local", "# local\nNODE_ENV=development");
    expect(env).toHaveLength(2);
    expect(env?.[1]).toContain("color:");

    expect(await highlightSource(".prettierrc", '{ "semi": true }')).toHaveLength(1);
  });

  it("colors a shell startup file, whose name is its whole extension", async () => {
    // `.zshrc` ends in `zshrc`, not in a suffix the extension table registered, so this is the file
    // that goes back to plain if the name table loses an entry.
    const lines = await highlightSource(".zshrc", "# prompt\nexport EDITOR=nvim");
    expect(lines).toHaveLength(2);
    expect(lines?.[0]).toContain("var(--marvis-syntax-token-comment)");
    expect(lines?.[1]).toContain("color:");
  });

  it("gives a name that asks for no grammar no highlighting, and no grammar a module path", async () => {
    for (const name of [
      "",
      "not-a-language",
      "../../etc/passwd",
      "shiki/langs/typescript.mjs",
      "javascript:alert(1)",
      "<img src=x onerror=alert(1)>",
    ]) {
      expect(await highlightSourceAs(name, "answer = 42")).toBeNull();
    }
    // Shiki ships no plaintext language, so the name that means "do not highlight" is one the
    // allowlist does not know, and the caller's own text is what stays on screen.
    expect(await highlightSourceAs(PLAIN_TEXT, "answer = 42")).toBeNull();
  });

  it("keeps only sanitized Shiki line markup", () => {
    const lines = sanitizeHighlightedHtml(
      '<pre><code><span class="line" onclick="alert(3)"><span style="color:#FF7B72">const</span><img src="x" onerror="alert(1)"><script>alert(2)</script></span></code></pre>',
    );

    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain("const");
    expect(lines[0]).not.toMatch(/<\/?(?:pre|code|img|script)\b/i);
    expect(lines[0]).not.toMatch(/on(?:click|error)=/i);
  });

  it("resolves a fence info string through the allowlist only", () => {
    const language = languageForFenceInfo("ts");
    expect(language).toBeDefined();
    expect(languageForFenceInfo("BASH")).toBe(languageForFenceInfo("bash"));
    expect(languageForFenceInfo('ts title="app.ts"')?.name).toBe(language?.name);

    for (const hostile of [
      "",
      "   ",
      "not-a-language",
      "../../etc/passwd",
      "shiki/langs/typescript.mjs",
      "javascript:alert(1)",
      "<img src=x onerror=alert(1)>",
    ]) {
      expect(languageForFenceInfo(hostile)).toBeUndefined();
    }
  });

  it("returns the colored body of a block without the pre Shiki wraps it in", async () => {
    const language = languageForFenceInfo("typescript");
    const first = highlightCodeBlock(language!, "const answer: number = 42;");
    const second = highlightCodeBlock(language!, "const answer: number = 42;");

    expect(second).toBe(first);
    const html = await first;
    expect(html).toContain('<span class="line">');
    expect(html).toContain("color:");
    expect(html).not.toMatch(/<\/?(?:pre|code)\b/i);
  });

  it("keeps a hostile code body as escaped text, never as markup", async () => {
    const language = languageForFenceInfo("html");
    const body = '<span onclick="alert(1)">x</span><span style="background:url(javascript:alert(2))">y</span>';
    const html = await highlightCodeBlock(language!, body);
    const root = new DOMParser().parseFromString(`<div>${html}</div>`, "text/html").body.firstElementChild;

    // Shiki tokenizes the body as text, so the only way it could become live markup is if the
    // fragment carried anything but a class and a color.
    for (const element of Array.from(root?.querySelectorAll("*") ?? [])) {
      expect(element.tagName).toBe("SPAN");
      for (const attribute of Array.from(element.attributes)) {
        expect(["class", "style"]).toContain(attribute.name);
        expect(attribute.value).not.toMatch(/(?:url|expression|javascript)\s*\(|[<>]/i);
      }
    }
    // The body survived as the exact text it was, which is what "escaped, not parsed" means.
    expect(root?.textContent).toBe(body);
  });
});
