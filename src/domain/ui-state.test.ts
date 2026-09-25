import { describe, expect, it } from "vitest";
import {
  DEFAULT_APP_LAYOUT,
  DEFAULT_CHECKOUT_UI_STATE,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  needsInspectorDrawer,
  resizeLayoutPanel,
  snapshotLayout,
  toggleFocusLayout,
  toggleLayoutVisibility,
} from "./ui-state";

describe("persisted UI state", () => {
  it("round-trips versioned global widths, visibility, collapse, and focus restore state", () => {
    const saved = {
      ...DEFAULT_APP_LAYOUT,
      sidebarWidth: 340,
      inspectorVisible: false,
      collapsedRepoIds: ["repo:/one"],
      focusSnapshot: {
        sidebarWidth: 280,
        inspectorWidth: 360,
        sidebarVisible: true,
        inspectorVisible: true,
        statusBarVisible: true,
      },
    };
    expect(normalizeAppLayout(JSON.parse(JSON.stringify(saved)))).toEqual(saved);
    expect(snapshotLayout(saved).sidebarWidth).toBe(340);
  });

  it("uses defaults for unknown versions and clamps corrupt dimensions", () => {
    expect(normalizeAppLayout({ version: 9 })).toEqual(DEFAULT_APP_LAYOUT);
    expect(normalizeAppLayout({ version: 1, sidebarWidth: 900, inspectorWidth: -1 })).toMatchObject({
      sidebarWidth: 380,
      inspectorWidth: 260,
    });
  });

  it("keeps expanded widths across collapse and clamps user resizing", () => {
    const resized = resizeLayoutPanel(DEFAULT_APP_LAYOUT, "sidebar", 345.4);
    const collapsed = toggleLayoutVisibility(resized, "sidebarVisible");
    const expanded = toggleLayoutVisibility(collapsed, "sidebarVisible");
    expect(resized.sidebarWidth).toBe(345);
    expect(collapsed.sidebarVisible).toBe(false);
    expect(expanded).toMatchObject({ sidebarVisible: true, sidebarWidth: 345 });
    expect(resizeLayoutPanel(resized, "inspector", 900).inspectorWidth).toBe(560);
  });

  it("uses an inspector drawer only when needed without mutating preferred layout", () => {
    const preferred = { ...DEFAULT_APP_LAYOUT, sidebarWidth: 380, inspectorWidth: 500 };
    expect(needsInspectorDrawer(preferred, 900)).toBe(true);
    expect(needsInspectorDrawer(preferred, 1400)).toBe(false);
    expect(preferred).toMatchObject({ sidebarWidth: 380, inspectorWidth: 500, inspectorVisible: true });
  });

  it("restores the full previous layout after focus mode", () => {
    const previous = { ...DEFAULT_APP_LAYOUT, sidebarWidth: 300, statusBarVisible: false };
    const focused = toggleFocusLayout(previous);
    expect(focused).toMatchObject({ sidebarVisible: false, inspectorVisible: false, statusBarVisible: false });
    expect(toggleFocusLayout(focused)).toEqual(previous);
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
