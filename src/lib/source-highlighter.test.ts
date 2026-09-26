// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { highlightSource, sanitizeHighlightedHtml } from "./source-highlighter";

describe("source highlighter", () => {
  it("highlights known source and reuses the path/content cache entry", async () => {
    const first = highlightSource("src/cache.ts", "const answer: number = 42;\nreturn answer;");
    const second = highlightSource("src/cache.ts", "const answer: number = 42;\nreturn answer;");

    expect(second).toBe(first);
    const lines = await first;
    expect(lines).toHaveLength(2);
    expect(lines?.[0]).toContain('class="line"');
    expect(lines?.[0]).toContain("color:");
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
});
