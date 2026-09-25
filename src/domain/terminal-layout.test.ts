import { describe, expect, it } from "vitest";
import type { Session } from "./workspace";
import {
  addSessionToLayout,
  createTerminalLayout,
  findSessionTab,
  firstSessionId,
  normalizeTerminalLayout,
  removeSessionFromLayout,
  reorderSession,
  resizeSplit,
  restoreTerminalLayout,
} from "./terminal-layout";

const session = (id: string, checkoutId = "checkout:first"): Session => ({
  id,
  checkoutId,
  type: "shell",
  name: id,
  createdAt: "now",
  status: "inactive",
});

describe("terminal layout", () => {
  it("persists nested splits and restores them without inventing live sessions", () => {
    let layout = createTerminalLayout([session("one")]);
    layout = addSessionToLayout(layout, session("two"), {
      targetSessionId: "one",
      direction: "horizontal",
      splitId: "split:one-two",
    });
    layout = addSessionToLayout(layout, session("three"), {
      targetSessionId: "two",
      direction: "vertical",
      splitId: "split:two-three",
    });

    const restored = restoreTerminalLayout(JSON.parse(JSON.stringify(layout)), [
      session("one"),
      session("two"),
      session("three"),
    ]);

    expect(restored).toEqual(layout);
    expect(restored.tabs).toHaveLength(1);
    expect(restored.tabs[0].root).toMatchObject({
      kind: "split",
      direction: "horizontal",
      second: {
        kind: "split",
        direction: "vertical",
        second: { kind: "session", sessionId: "three" },
      },
    });
  });

  it("drops removed checkout sessions and creates historical tabs for sessions missing in saved layout", () => {
    const layout = createTerminalLayout([session("first"), session("second")]);
    const restored = restoreTerminalLayout(
      { ...layout, tabs: [{ id: "broken", root: { kind: "session", sessionId: "removed" } }] },
      [session("second")],
    );

    expect(restored.tabs.map((tab) => firstSessionId(tab.root))).toEqual(["second"]);
    expect(restored.sessionOrder).toEqual(["second"]);
  });

  it("reorders sessions and collapses a split after one session is removed", () => {
    let layout = addSessionToLayout(createTerminalLayout([session("one")]), session("two"), {
      targetSessionId: "one",
      direction: "vertical",
      splitId: "split:one-two",
    });
    layout = reorderSession(layout, "two", "one");
    expect(layout.sessionOrder).toEqual(["two", "one"]);
    expect(findSessionTab(layout, "two")?.root.kind).toBe("split");

    layout = removeSessionFromLayout(layout, "two");
    expect(layout.tabs).toHaveLength(1);
    expect(layout.tabs[0].root).toEqual({ kind: "session", sessionId: "one" });
    expect(layout.sessionOrder).toEqual(["one"]);
  });

  it("keeps layout state isolated by checkout session membership", () => {
    const first = restoreTerminalLayout(createTerminalLayout([session("same-id", "checkout:first")]), [
      session("same-id", "checkout:first"),
    ]);
    const second = restoreTerminalLayout(createTerminalLayout([session("same-id", "checkout:second")]), [
      session("same-id", "checkout:second"),
    ]);
    expect(first.tabs).toEqual(second.tabs);
    expect(first).not.toBe(second);
  });

  it("resizes one nested split and clamps panes away from zero size", () => {
    let layout = addSessionToLayout(createTerminalLayout([session("one")]), session("two"), {
      targetSessionId: "one",
      direction: "horizontal",
      splitId: "outer",
    });
    layout = addSessionToLayout(layout, session("three"), {
      targetSessionId: "two",
      direction: "vertical",
      splitId: "inner",
    });

    layout = resizeSplit(layout, "inner", 0.91);
    const outer = layout.tabs[0].root;
    expect(outer).toMatchObject({ kind: "split", id: "outer", ratio: 0.5 });
    if (outer.kind !== "split" || outer.second.kind !== "split") throw new Error("nested layout expected");
    expect(outer.second.ratio).toBe(0.85);
  });

  it("normalizes persisted tabs and splits to one selected session", () => {
    let layout = createTerminalLayout([session("one"), session("two")]);
    layout = addSessionToLayout(layout, session("three"), {
      targetSessionId: "two",
      direction: "vertical",
      splitId: "split:legacy",
    });

    const normalized = normalizeTerminalLayout(layout, [session("one"), session("two"), session("three")], "one");

    expect(normalized).toEqual({
      activeTabId: "layout:one",
      tabs: [{ id: "layout:one", root: { kind: "session", sessionId: "one" } }],
      sessionOrder: ["one"],
    });
  });
});
