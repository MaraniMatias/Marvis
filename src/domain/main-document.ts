export type MainDocumentMode = "diff" | "view" | "code";
export type MainView = "terminal" | "document";

export interface MainDocument {
  checkoutId: string;
  path: string;
  source: "file" | "change";
  mode: MainDocumentMode;
}

export function resolveMainView(
  mainViews: Record<string, MainView>,
  checkoutId: string | null,
  document: MainDocument | null,
): MainView {
  return checkoutId && document?.checkoutId === checkoutId && mainViews[checkoutId] === "document"
    ? "document"
    : "terminal";
}
