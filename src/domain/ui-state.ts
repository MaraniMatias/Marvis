import type { MainDocument } from "./main-document";

export interface LayoutSnapshot {
  sidebarWidth: number;
  inspectorWidth: number;
  sidebarVisible: boolean;
  inspectorVisible: boolean;
}

export interface AppLayoutState extends LayoutSnapshot {
  version: 1;
  focusSnapshot: LayoutSnapshot | null;
  collapsedRepoIds: string[];
  reduceTransparency: boolean;
}

export interface CheckoutUiState {
  version: 1;
  document: MainDocument | null;
  mainView: "terminal" | "document";
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

export const DEFAULT_APP_LAYOUT: AppLayoutState = {
  version: 1,
  sidebarWidth: 260,
  inspectorWidth: 320,
  sidebarVisible: true,
  inspectorVisible: true,
  focusSnapshot: null,
  collapsedRepoIds: [],
  reduceTransparency: false,
};

export const DEFAULT_CHECKOUT_UI_STATE: CheckoutUiState = {
  version: 1,
  document: null,
  mainView: "terminal",
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

function snapshot(value: unknown): LayoutSnapshot | null {
  if (!isRecord(value)) return null;
  return {
    sidebarWidth: boundedNumber(value.sidebarWidth, 260, 220, 380),
    inspectorWidth: boundedNumber(value.inspectorWidth, 320, 260, 560),
    sidebarVisible: typeof value.sidebarVisible === "boolean" ? value.sidebarVisible : true,
    inspectorVisible: typeof value.inspectorVisible === "boolean" ? value.inspectorVisible : true,
  };
}

export function normalizeAppLayout(value: unknown): AppLayoutState {
  if (!isRecord(value) || value.version !== 1) return { ...DEFAULT_APP_LAYOUT };
  const collapsedRepoIds = Array.isArray(value.collapsedRepoIds)
    ? value.collapsedRepoIds.filter((id): id is string => typeof id === "string" && id.length <= 4096).slice(0, 1000)
    : [];
  return {
    version: 1,
    ...(snapshot(value) ?? DEFAULT_APP_LAYOUT),
    focusSnapshot: value.focusSnapshot === null ? null : snapshot(value.focusSnapshot),
    collapsedRepoIds: [...new Set(collapsedRepoIds)],
    reduceTransparency: typeof value.reduceTransparency === "boolean" ? value.reduceTransparency : false,
  };
}

export function normalizeCheckoutUiState(value: unknown): CheckoutUiState {
  if (!isRecord(value) || value.version !== 1) return { ...DEFAULT_CHECKOUT_UI_STATE };
  const rawDocument = value.document;
  const document =
    isRecord(rawDocument) &&
    safePath(rawDocument.path) &&
    typeof rawDocument.checkoutId === "string" &&
    (rawDocument.source === "file" || rawDocument.source === "change") &&
    (rawDocument.mode === "diff" || rawDocument.mode === "view" || rawDocument.mode === "code")
      ? ({
          checkoutId: rawDocument.checkoutId,
          path: rawDocument.path,
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

export function snapshotLayout(layout: AppLayoutState): LayoutSnapshot {
  const { sidebarWidth, inspectorWidth, sidebarVisible, inspectorVisible } = layout;
  return { sidebarWidth, inspectorWidth, sidebarVisible, inspectorVisible };
}

export function toggleFocusLayout(layout: AppLayoutState): AppLayoutState {
  if (layout.focusSnapshot) return { ...layout, ...layout.focusSnapshot, focusSnapshot: null };
  return {
    ...layout,
    focusSnapshot: snapshotLayout(layout),
    sidebarVisible: false,
    inspectorVisible: false,
  };
}

export function toggleLayoutVisibility(
  layout: AppLayoutState,
  key: "sidebarVisible" | "inspectorVisible",
): AppLayoutState {
  if (layout.focusSnapshot) {
    const restored = { ...layout.focusSnapshot, [key]: !layout.focusSnapshot[key] };
    return { ...layout, ...restored, focusSnapshot: null };
  }
  return { ...layout, [key]: !layout[key] };
}

export function resizeLayoutPanel(
  layout: AppLayoutState,
  panel: "sidebar" | "inspector",
  width: number,
): AppLayoutState {
  if (!Number.isFinite(width)) return layout;
  return panel === "sidebar"
    ? { ...layout, sidebarWidth: Math.round(Math.min(380, Math.max(220, width))) }
    : { ...layout, inspectorWidth: Math.round(Math.min(560, Math.max(260, width))) };
}

export function needsInspectorDrawer(layout: AppLayoutState, viewportWidth: number): boolean {
  return (
    layout.inspectorVisible &&
    viewportWidth < (layout.sidebarVisible ? layout.sidebarWidth : 0) + layout.inspectorWidth + 430
  );
}
