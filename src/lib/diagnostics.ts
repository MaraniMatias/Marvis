import { invoke } from "@tauri-apps/api/core";
import type { App } from "vue";

export const DIAGNOSTIC_CATEGORIES = [
  "frontend_vue_error",
  "frontend_window_error",
  "frontend_unhandled_rejection",
  "ui_writes_deadline",
  "exit_sweep_deadline",
] as const;

export type DiagnosticCategory = (typeof DIAGNOSTIC_CATEGORIES)[number];
export type DiagnosticReporter = (category: DiagnosticCategory, cause?: unknown) => Promise<void>;

const REPORT_INTERVAL_MS = 60_000;
const ERROR_DEDUPLICATION_MS = 1_000;

export function createDiagnosticReporter(
  invokeDiagnostic: (category: DiagnosticCategory) => Promise<unknown>,
  now: () => number = () => performance.now(),
): DiagnosticReporter {
  const lastReported = new Map<DiagnosticCategory, number>();
  const seenErrors = new WeakMap<object, number>();

  return (category, cause) => {
    const timestamp = now();
    if ((typeof cause === "object" && cause !== null) || typeof cause === "function") {
      const lastSeen = seenErrors.get(cause);
      if (lastSeen !== undefined && timestamp - lastSeen < ERROR_DEDUPLICATION_MS) return Promise.resolve();
      seenErrors.set(cause, timestamp);
    }

    const last = lastReported.get(category);
    if (last !== undefined && timestamp - last < REPORT_INTERVAL_MS) return Promise.resolve();
    lastReported.set(category, timestamp);

    return Promise.resolve()
      .then(() => invokeDiagnostic(category))
      .then(
        () => undefined,
        () => undefined,
      );
  };
}

export const reportFrontendDiagnostic = createDiagnosticReporter((category) =>
  invoke<void>("frontend_diagnostic", { category }),
);

export function installFrontendErrorHandlers(app: App, report = reportFrontendDiagnostic): () => void {
  app.config.errorHandler = (error, _instance, info) => {
    if (import.meta.env.DEV) console.error("[Vue error]", error, info);
    void report("frontend_vue_error", error);
  };

  const onError = (event: ErrorEvent) => {
    void report("frontend_window_error", event.error);
  };
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    void report("frontend_unhandled_rejection", event.reason);
  };
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onUnhandledRejection);

  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
  };
}
