import { describe, expect, it } from "vitest";
import {
  DEFAULT_APP_LAYOUT,
  DEFAULT_CHECKOUT_UI_STATE,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  needsInspectorDrawer,
  resizeLayoutPanel,
} from "./ui-state";

describe("persisted UI state", () => {
  it("keeps the marvis default panel widths", () => {
    expect(DEFAULT_APP_LAYOUT).toEqual({ version: 1, sidebarWidth: 240, inspectorWidth: 280 });
  });

  it("round-trips the saved widths and drops the fields the layout no longer has", () => {
    const saved = { ...DEFAULT_APP_LAYOUT, sidebarWidth: 340, inspectorWidth: 420 };
    expect(normalizeAppLayout(JSON.parse(JSON.stringify(saved)))).toEqual(saved);
    expect(
      normalizeAppLayout({
        ...saved,
        sidebarVisible: false,
        focusSnapshot: { sidebarWidth: 300 },
        collapsedRepoIds: ["repo:/one"],
        reduceTransparency: true,
      }),
    ).toEqual(saved);
  });

  it("uses defaults for unknown versions and clamps corrupt dimensions", () => {
    expect(normalizeAppLayout({ version: 9 })).toEqual(DEFAULT_APP_LAYOUT);
    expect(normalizeAppLayout({ version: 1, sidebarWidth: 900, inspectorWidth: -1 })).toEqual({
      version: 1,
      sidebarWidth: 500,
      inspectorWidth: 200,
    });
    expect(normalizeAppLayout({ version: 1 })).toEqual(DEFAULT_APP_LAYOUT);
  });

  it("clamps user resizing to the supported range", () => {
    const resized = resizeLayoutPanel(DEFAULT_APP_LAYOUT, "sidebar", 345.4);
    expect(resized.sidebarWidth).toBe(345);
    expect(resized.inspectorWidth).toBe(DEFAULT_APP_LAYOUT.inspectorWidth);
    expect(resizeLayoutPanel(resized, "sidebar", 10).sidebarWidth).toBe(240);
    expect(resizeLayoutPanel(resized, "inspector", 900).inspectorWidth).toBe(480);
    expect(resizeLayoutPanel(resized, "inspector", Number.NaN)).toEqual(resized);
  });

  it("asks for the inspector drawer only when the main panel would not fit", () => {
    const defaults = DEFAULT_APP_LAYOUT;
    expect(needsInspectorDrawer(defaults, 949)).toBe(true);
    expect(needsInspectorDrawer(defaults, 950)).toBe(false);
    const widened = resizeLayoutPanel(defaults, "sidebar", 500);
    expect(needsInspectorDrawer(widened, 1210)).toBe(false);
    expect(needsInspectorDrawer(widened, 1209)).toBe(true);
    expect(widened).toMatchObject({ sidebarWidth: 500, inspectorWidth: 280 });
  });

  it("restores checkout document and inspector state while rejecting unsafe paths", () => {
    const saved = {
      ...DEFAULT_CHECKOUT_UI_STATE,
      document: { checkoutId: "checkout:/repo", path: "docs/guide.md", source: "file", mode: "view" },
      mainView: "document",
      inspectorTab: "changes",
      selectedFilePath: "docs/guide.md",
      selectedChangePath: "src/main.rs",
      expandedDirectories: ["docs", "src"],
      filesScrollTop: 640,
      changesScrollTop: 480,
      documentScrollTop: 320,
      diffScrollTop: 900,
    };
    expect(normalizeCheckoutUiState(JSON.parse(JSON.stringify(saved)))).toEqual(saved);
    expect(
      normalizeCheckoutUiState({
        version: 1,
        document: { ...saved.document, path: "../../outside" },
        selectedFilePath: "../outside",
        expandedDirectories: ["safe", "a/../outside"],
      }),
    ).toMatchObject({ document: null, selectedFilePath: null, expandedDirectories: ["safe"] });
  });
});
