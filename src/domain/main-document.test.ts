import { describe, expect, it } from "vitest";
import { resolveMainView } from "./main-document";

describe("checkout-scoped main views", () => {
  it("restores each checkout's saved center view when selection moves away and back", () => {
    const mainViews = { "checkout:docs": "document", "checkout:terminal": "terminal" } as const;
    const document = { checkoutId: "checkout:docs", path: "README.md", source: "file", mode: "view" } as const;
    expect(resolveMainView(mainViews, "checkout:docs", document)).toBe("document");
    expect(resolveMainView(mainViews, "checkout:terminal", null)).toBe("terminal");
    expect(resolveMainView(mainViews, "checkout:docs", document)).toBe("document");
  });
});
