export type MainDocumentMode = "diff" | "view" | "code";

export interface MainDocument {
  checkoutId: string;
  path: string;
  source: "file" | "change";
  mode: MainDocumentMode;
}
