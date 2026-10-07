// @vitest-environment happy-dom
import { createApp } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createDiagnosticReporter, DIAGNOSTIC_CATEGORIES, installFrontendErrorHandlers } from "./diagnostics";
import type { DiagnosticCategory } from "./diagnostics";

const testRoot = { render: () => null };

describe("frontend diagnostics", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it("sends only whitelisted categories and throttles each category", async () => {
    expect(DIAGNOSTIC_CATEGORIES).toEqual([
      "frontend_vue_error",
      "frontend_window_error",
      "frontend_unhandled_rejection",
      "ui_writes_deadline",
      "exit_sweep_deadline",
    ]);

    let now = 1_000;
    const invokeDiagnostic = vi.fn(async (category: DiagnosticCategory) => {
      expect(DIAGNOSTIC_CATEGORIES).toContain(category);
    });
    const report = createDiagnosticReporter(invokeDiagnostic, () => now);
    const privateError = new Error("private message must not cross the bridge");

    await Promise.all([report("frontend_vue_error", privateError), report("frontend_vue_error", privateError)]);
    await report("frontend_window_error", new Error("a different private error"));
    now += 60_000;
    await report("frontend_vue_error", privateError);

    expect(invokeDiagnostic.mock.calls.map(([category]) => category)).toEqual([
      "frontend_vue_error",
      "frontend_window_error",
      "frontend_vue_error",
    ]);
    expect(invokeDiagnostic.mock.calls[0]).toHaveLength(1);
  });

  it("logs Vue errors in development without sending their payload to Rust", async () => {
    vi.stubEnv("DEV", true);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const invokeDiagnostic = vi.fn(async () => undefined);
    const app = createApp(testRoot);
    const dispose = installFrontendErrorHandlers(app, createDiagnosticReporter(invokeDiagnostic));
    const error = new TypeError("private Vue error");

    app.config.errorHandler?.(error, null, "setup");
    await Promise.resolve();
    dispose();

    expect(consoleError).toHaveBeenCalledWith("[Vue error]", error, "setup");
    expect(invokeDiagnostic.mock.calls).toEqual([["frontend_vue_error"]]);
  });

  it("does not echo Vue errors in release", async () => {
    vi.stubEnv("DEV", false);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const invokeDiagnostic = vi.fn(async () => undefined);
    const app = createApp(testRoot);
    const dispose = installFrontendErrorHandlers(app, createDiagnosticReporter(invokeDiagnostic));
    const error = new TypeError("private Vue error");

    app.config.errorHandler?.(error, null, "setup");
    await Promise.resolve();
    dispose();

    expect(consoleError).not.toHaveBeenCalled();
    expect(invokeDiagnostic.mock.calls).toEqual([["frontend_vue_error"]]);
  });

  it("reports Vue and window failures without suppressing defaults or duplicating one error", async () => {
    vi.stubEnv("DEV", false);
    const reported: DiagnosticCategory[] = [];
    const report = createDiagnosticReporter(async (category) => {
      reported.push(category);
    });
    const app = createApp(testRoot);
    const dispose = installFrontendErrorHandlers(app, report);
    const sharedError = new Error("private stack must not be logged");

    app.config.errorHandler?.(sharedError, null, "setup");
    const duplicate = new ErrorEvent("error", { cancelable: true, error: sharedError });
    expect(window.dispatchEvent(duplicate)).toBe(true);
    expect(duplicate.defaultPrevented).toBe(false);

    const windowError = new ErrorEvent("error", { cancelable: true, error: new Error("private") });
    expect(window.dispatchEvent(windowError)).toBe(true);
    const rejection = new Event("unhandledrejection", { cancelable: true }) as PromiseRejectionEvent;
    Object.defineProperty(rejection, "reason", { value: "private rejection" });
    expect(window.dispatchEvent(rejection)).toBe(true);
    expect(rejection.defaultPrevented).toBe(false);

    await Promise.resolve();
    dispose();
    expect(reported).toEqual(["frontend_vue_error", "frontend_window_error", "frontend_unhandled_rejection"]);
  });

  it("does not echo an invoke failure", async () => {
    const report = createDiagnosticReporter(async () => {
      throw new Error("private invoke failure");
    });

    await expect(report("frontend_vue_error")).resolves.toBeUndefined();
  });
});
