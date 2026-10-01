import type { MainDocument, MainViewState } from "./main-document";

/**
 * How the terminal's own scrollbar is drawn, in the order a menu lists them.
 *
 * `hidden` is the default and the terminal is full-bleed: the scrollback is there for when it is
 * asked for, and a permanent strip down the right edge of every pane is a column of dead pixels
 * the fit has to work around. `auto` is what a desktop terminal does — the thumb shows up when
 * something moves and gets out of the way when nothing does — and `always` is for the people who
 * scroll by dragging it.
 */
export const TERMINAL_SCROLLBAR_MODES = ["hidden", "auto", "always"] as const;
export type TerminalScrollbarMode = (typeof TERMINAL_SCROLLBAR_MODES)[number];

export interface AppLayoutState {
  version: 3;
  mode: "focus" | "split";
  sidebarWidth: number;
  inspectorWidth: number;
  previewWidth: number;
  terminalScrollbar: TerminalScrollbarMode;
}

export interface CheckoutUiState extends MainViewState {
  version: 1;
  inspectorTab: "files" | "changes";
  selectedFilePath: string | null;
  selectedChangePath: string | null;
  expandedDirectories: string[];
  filesScrollTop: number;
  changesScrollTop: number;
  documentScrollTop: number;
  documentScrollLeft: number;
  diffScrollTop: number;
}

export const SIDEBAR_WIDTH_LIMITS = { min: 240, max: 500 } as const;
export const INSPECTOR_WIDTH_LIMITS = { min: 200, max: 480 } as const;
export const PREVIEW_WIDTH_LIMITS = { min: 260, max: 900 } as const;
/** What the main panel needs before the inspector can no longer sit beside it. */
const MIN_MAIN_WIDTH = 430;

export const DEFAULT_APP_LAYOUT: AppLayoutState = {
  version: 3,
  mode: "focus",
  sidebarWidth: 240,
  inspectorWidth: 280,
  previewWidth: 360,
  terminalScrollbar: "hidden",
};

export const DEFAULT_CHECKOUT_UI_STATE: CheckoutUiState = {
  version: 1,
  document: null,
  mainView: "terminal",
  diffAllFiles: false,
  inspectorTab: "files",
  selectedFilePath: null,
  selectedChangePath: null,
  expandedDirectories: [],
  filesScrollTop: 0,
  changesScrollTop: 0,
  documentScrollTop: 0,
  documentScrollLeft: 0,
  diffScrollTop: 0,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function boundedNumber(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : fallback;
}

function safePath(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0 || value.length > 4096 || value.startsWith("/")) return false;
  return value.split(/[\\/]/).every((part) => part !== ".." && part !== "." && part.length > 0);
}

function safeReviewPath(value: unknown): value is string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 4096 ||
    value.startsWith("/") ||
    /^[A-Za-z]:/.test(value) ||
    !value.endsWith(".md") ||
    value.includes("\\") ||
    value.includes("\0")
  )
    return false;
  return value.split("/").every((part) => part !== ".." && part !== "." && part.length > 0);
}

export function normalizeAppLayout(value: unknown): AppLayoutState {
  if (!isRecord(value) || value.version !== 3) return { ...DEFAULT_APP_LAYOUT };
  return {
    version: 3,
    mode: value.mode === "split" ? "split" : "focus",
    sidebarWidth: boundedNumber(
      value.sidebarWidth,
      DEFAULT_APP_LAYOUT.sidebarWidth,
      SIDEBAR_WIDTH_LIMITS.min,
      SIDEBAR_WIDTH_LIMITS.max,
    ),
    inspectorWidth: boundedNumber(
      value.inspectorWidth,
      DEFAULT_APP_LAYOUT.inspectorWidth,
      INSPECTOR_WIDTH_LIMITS.min,
      INSPECTOR_WIDTH_LIMITS.max,
    ),
    previewWidth: boundedNumber(
      value.previewWidth,
      DEFAULT_APP_LAYOUT.previewWidth,
      PREVIEW_WIDTH_LIMITS.min,
      PREVIEW_WIDTH_LIMITS.max,
    ),
    // Anything that is not one of the three is a build that no longer exists or a hand-edited
    // file, and the scrollbar is a preference rather than state worth refusing a layout over: the
    // one this build knows how to draw is a better answer than a pane that has no scrollbar at all.
    terminalScrollbar: TERMINAL_SCROLLBAR_MODES.includes(value.terminalScrollbar as TerminalScrollbarMode)
      ? (value.terminalScrollbar as TerminalScrollbarMode)
      : DEFAULT_APP_LAYOUT.terminalScrollbar,
  };
}

export function normalizeCheckoutUiState(value: unknown): CheckoutUiState {
  if (!isRecord(value) || value.version !== 1) return { ...DEFAULT_CHECKOUT_UI_STATE };
  const rawDocument = value.document;
  const document =
    isRecord(rawDocument) &&
    typeof rawDocument.path === "string" &&
    (rawDocument.origin === "review" || rawDocument.origin === "checkout") &&
    (rawDocument.origin === "review" ? safeReviewPath(rawDocument.path) : safePath(rawDocument.path)) &&
    typeof rawDocument.checkoutId === "string" &&
    (rawDocument.source === "file" || rawDocument.source === "change") &&
    (rawDocument.mode === "diff" || rawDocument.mode === "view" || rawDocument.mode === "code") &&
    (rawDocument.origin !== "review" || (rawDocument.source === "file" && rawDocument.mode !== "diff"))
      ? ({
          checkoutId: rawDocument.checkoutId,
          path: rawDocument.path,
          origin: rawDocument.origin,
          source: rawDocument.source,
          mode: rawDocument.mode,
        } satisfies MainDocument)
      : null;
  const expandedDirectories = Array.isArray(value.expandedDirectories)
    ? value.expandedDirectories.filter(safePath).slice(0, 2000)
    : [];
  return {
    version: 1,
    document,
    mainView: value.mainView === "document" ? "document" : "terminal",
    diffAllFiles: value.diffAllFiles === true,
    inspectorTab: value.inspectorTab === "changes" ? "changes" : "files",
    selectedFilePath: safePath(value.selectedFilePath) ? value.selectedFilePath : null,
    selectedChangePath: safePath(value.selectedChangePath) ? value.selectedChangePath : null,
    expandedDirectories: [...new Set(expandedDirectories)],
    filesScrollTop: boundedNumber(value.filesScrollTop, 0, 0, 10_000_000),
    changesScrollTop: boundedNumber(value.changesScrollTop, 0, 0, 10_000_000),
    documentScrollTop: boundedNumber(value.documentScrollTop, 0, 0, 10_000_000),
    documentScrollLeft: boundedNumber(value.documentScrollLeft, 0, 0, 10_000_000),
    diffScrollTop: boundedNumber(value.diffScrollTop, 0, 0, 10_000_000),
  };
}

export function resizeLayoutPanel(
  layout: AppLayoutState,
  panel: "sidebar" | "inspector" | "preview",
  width: number,
): AppLayoutState {
  if (!Number.isFinite(width)) return layout;
  const limits =
    panel === "sidebar" ? SIDEBAR_WIDTH_LIMITS : panel === "inspector" ? INSPECTOR_WIDTH_LIMITS : PREVIEW_WIDTH_LIMITS;
  const clamped = Math.round(Math.min(limits.max, Math.max(limits.min, width)));
  if (panel === "sidebar") return { ...layout, sidebarWidth: clamped };
  return panel === "inspector" ? { ...layout, inspectorWidth: clamped } : { ...layout, previewWidth: clamped };
}

export function needsInspectorDrawer(layout: AppLayoutState, viewportWidth: number): boolean {
  return viewportWidth < layout.sidebarWidth + layout.inspectorWidth + MIN_MAIN_WIDTH;
}
