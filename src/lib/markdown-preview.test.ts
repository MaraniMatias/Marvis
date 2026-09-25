// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { attachMarkdownImages, renderMarkdownPreview } from "./markdown-preview";

describe("Markdown preview", () => {
  it("renders GFM tables, task lists, and highlighted fenced code", () => {
    const preview = renderMarkdownPreview(
      "| name | value |\n| --- | --- |\n| one | two |\n\n- [x] done\n- [ ] later\n\n```js\nconst answer = 42;\n```",
      "docs/readme.md",
    );

    expect(preview.html).toContain("<table>");
    expect(preview.html).toContain('type="checkbox"');
    expect(preview.html).toContain("checked");
    expect(preview.html).toContain('class="hljs-keyword"');
  });

  it("SEC-04 sanitizes scripts, event handlers, and javascript URLs", () => {
    const preview = renderMarkdownPreview(
      '<script>alert(1)</script>\n<img src="x" onerror="alert(2)">\n[bad](javascript:alert(3))\n![bad](javascript:alert(4))',
      "readme.md",
    );

    const document = new DOMParser().parseFromString(preview.html, "text/html");
    expect(document.querySelector("script, img[onerror], a[href^='javascript:']")).toBeNull();
    expect(preview.images).toEqual([]);
  });

  it("only asks the backend for contained relative images and attaches trusted data URLs", () => {
    const preview = renderMarkdownPreview(
      "![local](../images/pic.png) ![repeat](../images/pic.png) ![escape](../../../secret.png) ![remote](https://example.com/a.png)",
      "docs/guide/readme.md",
    );

    expect(preview.images).toEqual([{ source: "../images/pic.png", path: "docs/images/pic.png" }]);
    expect(preview.html).not.toContain("https://example.com");
    const html = attachMarkdownImages(
      preview.html,
      preview.images.map((image) => image.path),
      new Map([["docs/images/pic.png", { mimeType: "image/png", dataBase64: "iVBORw0KGgo=" }]]),
    );
    expect(html).toContain('src="data:image/png;base64,iVBORw0KGgo="');
    expect(html).not.toContain("data-marvis-image");
  });
});
