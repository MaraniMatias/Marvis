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
    expect(DEFAULT_APP_LAYOUT).toEqual({
      version: 5,
      mode: "focus",
      sidebarWidth: 240,
      inspectorWidth: 280,
      previewWidth: 360,
    });
  });

  it("round-trips the saved widths and drops the fields the layout no longer has", () => {
    const saved = {
      ...DEFAULT_APP_LAYOUT,
      mode: "split" as const,
      sidebarWidth: 340,
      inspectorWidth: 420,
      previewWidth: 720,
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
    // A layout written before the preferences moved to `~/.marvis/config.yml` is refused whole: it
    // is a shape this build cannot read, and honouring the widths out of it anyway would hand back
    // a window somebody had already arranged around a scale that is gone.
    expect(
      normalizeAppLayout({
        version: 4,
        mode: "split",
        sidebarWidth: 345,
        inspectorWidth: 450,
        previewWidth: 720,
        terminalScrollbar: "auto",
        zoom: 1.2,
      }),
    ).toEqual(DEFAULT_APP_LAYOUT);
    expect(normalizeAppLayout({ version: 5, sidebarWidth: 900, inspectorWidth: -1, previewWidth: 1200 })).toEqual({
      version: 5,
      mode: "focus",
      sidebarWidth: 500,
      inspectorWidth: 200,
      previewWidth: 900,
    });
    expect(normalizeAppLayout({ version: 5 })).toEqual(DEFAULT_APP_LAYOUT);
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
