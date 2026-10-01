import { describe, expect, it } from "vitest";
import {
  DEFAULT_APP_LAYOUT,
  DEFAULT_CHECKOUT_UI_STATE,
  TERMINAL_SCROLLBAR_MODES,
  normalizeAppLayout,
  normalizeCheckoutUiState,
  needsInspectorDrawer,
  resizeLayoutPanel,
} from "./ui-state";
import { ZOOM_STEPS } from "./zoom";

describe("persisted UI state", () => {
  it("keeps the marvis default panel widths", () => {
    expect(DEFAULT_APP_LAYOUT).toEqual({
      version: 4,
      mode: "focus",
      sidebarWidth: 240,
      inspectorWidth: 280,
      previewWidth: 360,
      terminalScrollbar: "hidden",
      zoom: 1,
    });
  });

  it("round-trips the saved widths and drops the fields the layout no longer has", () => {
    const saved = {
      ...DEFAULT_APP_LAYOUT,
      mode: "split" as const,
      sidebarWidth: 340,
      inspectorWidth: 420,
      previewWidth: 720,
      terminalScrollbar: "auto" as const,
      zoom: 1.2 as const,
    };
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
    expect(normalizeAppLayout({ version: 1, mode: "split", sidebarWidth: 400 })).toEqual(DEFAULT_APP_LAYOUT);
    expect(normalizeAppLayout({ version: 9 })).toEqual(DEFAULT_APP_LAYOUT);
    // The layout a version-3 build wrote is refused whole: it has no scale in it, and reading the
    // widths out of it anyway would hand back a window somebody had already arranged.
    expect(normalizeAppLayout({ version: 3, sidebarWidth: 345, inspectorWidth: 450, previewWidth: 720 })).toEqual(
      DEFAULT_APP_LAYOUT,
    );
    expect(normalizeAppLayout({ version: 4, sidebarWidth: 900, inspectorWidth: -1, previewWidth: 1200 })).toEqual({
      version: 4,
      mode: "focus",
      sidebarWidth: 500,
      inspectorWidth: 200,
      previewWidth: 900,
      terminalScrollbar: "hidden",
      zoom: 1,
    });
    expect(normalizeAppLayout({ version: 4 })).toEqual(DEFAULT_APP_LAYOUT);
  });

  it("keeps a scale it can draw and snaps one it cannot to the nearest step", () => {
    for (const zoom of ZOOM_STEPS) {
      expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, zoom }).zoom).toBe(zoom);
    }
    // The scale is a preference for the same reason the scrollbar is: a factor from a build with a
    // different range in it is still an answer, and the nearest step is what this build can draw.
    expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, zoom: 2.4 }).zoom).toBe(1.5);
    expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, zoom: 0.2 }).zoom).toBe(0.8);
    expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, zoom: "big" }).zoom).toBe(1);
  });

  it("keeps a scrollbar mode it knows and falls back to hidden for one it does not", () => {
    for (const mode of TERMINAL_SCROLLBAR_MODES) {
      expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, terminalScrollbar: mode }).terminalScrollbar).toBe(mode);
    }
    // The scrollbar is a preference and not state worth refusing a whole layout over, so only this
    // field is replaced: a name from a build that no longer exists leaves the panels as they were.
    expect(normalizeAppLayout({ ...DEFAULT_APP_LAYOUT, sidebarWidth: 400, terminalScrollbar: "warp" })).toMatchObject({
      sidebarWidth: 400,
      terminalScrollbar: "hidden",
    });
  });

  it("clamps user resizing to the supported range", () => {
    const resized = resizeLayoutPanel(DEFAULT_APP_LAYOUT, "sidebar", 345.4);
    expect(resized.sidebarWidth).toBe(345);
    expect(resized.inspectorWidth).toBe(DEFAULT_APP_LAYOUT.inspectorWidth);
    expect(resizeLayoutPanel(resized, "sidebar", 10).sidebarWidth).toBe(240);
    expect(resizeLayoutPanel(resized, "inspector", 900).inspectorWidth).toBe(480);
    expect(resizeLayoutPanel(resized, "preview", 100).previewWidth).toBe(260);
    expect(resizeLayoutPanel(resized, "preview", 1000).previewWidth).toBe(900);
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
      document: {
        checkoutId: "checkout:/repo",
        path: "docs/guide.md",
        origin: "checkout",
        source: "file",
        mode: "view",
      },
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

  it("defaults UI flags when absent, and drops documents without a known origin", () => {
    // The panel flags have defaults, but a document without its required root is not guessable.
    expect(
      normalizeCheckoutUiState({
        version: 1,
        mainView: "document",
        document: {
          checkoutId: "checkout:/repo",
          path: "docs/guide.md",
          origin: "checkout",
          source: "file",
          mode: "view",
        },
        inspectorTab: "files",
      }),
    ).toMatchObject({
      mainView: "document",
      diffAllFiles: false,
      document: { path: "docs/guide.md", origin: "checkout", source: "file", mode: "view" },
    });
    expect(
      normalizeCheckoutUiState({
        version: 1,
        mainView: "document",
        document: { checkoutId: "checkout:/repo", path: "docs/guide.md", source: "file", mode: "view" },
      }),
    ).toMatchObject({ mainView: "document", document: null });
    expect(
      normalizeCheckoutUiState({
        version: 1,
        mainView: "document",
        document: {
          checkoutId: "checkout:/repo",
          path: "2026-03-14-1532.md",
          origin: "review",
          source: "file",
          mode: "view",
        },
      }),
    ).toMatchObject({ document: { origin: "review", path: "2026-03-14-1532.md" } });
    // An unreadable mainView or flag falls to the terminal rather than failing the load.
    expect(normalizeCheckoutUiState({ version: 1, mainView: "diff", diffAllFiles: "yes" })).toMatchObject({
      mainView: "terminal",
      diffAllFiles: false,
    });
    expect(normalizeCheckoutUiState({ version: 1, diffAllFiles: true })).toMatchObject({ diffAllFiles: true });
  });
});
