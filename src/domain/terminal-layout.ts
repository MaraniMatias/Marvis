import type { Session } from "./workspace";

export type SplitDirection = "horizontal" | "vertical";

export type TerminalLayoutNode =
  | { kind: "session"; sessionId: string }
  | {
      kind: "split";
      id: string;
      direction: SplitDirection;
      ratio: number;
      first: TerminalLayoutNode;
      second: TerminalLayoutNode;
    };

export interface TerminalLayoutTab {
  id: string;
  root: TerminalLayoutNode;
}

export interface CheckoutTerminalLayout {
  activeTabId: string | null;
  tabs: TerminalLayoutTab[];
  sessionOrder: string[];
}

export function createTerminalLayout(sessions: Session[] = []): CheckoutTerminalLayout {
  const tabs = sessions.map((session) => ({
    id: `layout:${session.id}`,
    root: { kind: "session", sessionId: session.id } as TerminalLayoutNode,
  }));
  return {
    activeTabId: tabs[0]?.id ?? null,
    tabs,
    sessionOrder: sessions.map((session) => session.id),
  };
}

export function restoreTerminalLayout(value: unknown, sessions: Session[]): CheckoutTerminalLayout {
  const validIds = new Set(sessions.map((session) => session.id));
  const seen = new Set<string>();
  const raw = value && typeof value === "object" ? (value as Partial<CheckoutTerminalLayout>) : {};
  const tabs: TerminalLayoutTab[] = [];

  function restoreNode(node: unknown): TerminalLayoutNode | null {
    if (!node || typeof node !== "object") return null;
    const value = node as Partial<TerminalLayoutNode>;
    if (value.kind === "session") {
      const sessionId = value.sessionId;
      if (typeof sessionId !== "string" || !validIds.has(sessionId) || seen.has(sessionId)) return null;
      seen.add(sessionId);
      return { kind: "session", sessionId };
    }
    if (value.kind !== "split") return null;
    const split = value as Extract<TerminalLayoutNode, { kind: "split" }>;
    const first = restoreNode(split.first);
    const second = restoreNode(split.second);
    if (!first) return second;
    if (!second) return first;
    const direction = split.direction === "vertical" ? "vertical" : "horizontal";
    const ratio = typeof split.ratio === "number" && Number.isFinite(split.ratio) ? split.ratio : 0.5;
    return {
      kind: "split",
      id: typeof split.id === "string" ? split.id : `split:${tabs.length}`,
      direction,
      ratio: Math.max(0.15, Math.min(0.85, ratio)),
      first,
      second,
    };
  }

  if (Array.isArray(raw.tabs)) {
    for (const rawTab of raw.tabs) {
      if (!rawTab || typeof rawTab !== "object") continue;
      const tab = rawTab as TerminalLayoutTab;
      const root = restoreNode(tab.root);
      if (root) tabs.push({ id: typeof tab.id === "string" ? tab.id : `layout:${firstSessionId(root)}`, root });
    }
  }

  for (const session of sessions) {
    if (seen.has(session.id)) continue;
    const tab = createTerminalLayout([session]).tabs[0];
    tabs.push(tab);
    seen.add(session.id);
  }

  const tabIds = new Set<string>();
  for (const tab of tabs) {
    if (tabIds.has(tab.id)) tab.id = `layout:${firstSessionId(tab.root)}`;
    tabIds.add(tab.id);
  }
  const requestedOrder = Array.isArray(raw.sessionOrder) ? raw.sessionOrder : [];
  const sessionOrder = requestedOrder.filter((id): id is string => typeof id === "string" && validIds.has(id));
  const orderSet = new Set(sessionOrder);
  for (const session of sessions) {
    if (!orderSet.has(session.id)) sessionOrder.push(session.id);
  }
  const activeTabId = tabs.some((tab) => tab.id === raw.activeTabId) ? raw.activeTabId! : (tabs[0]?.id ?? null);
  return { activeTabId, tabs, sessionOrder: [...new Set(sessionOrder)] };
}

export function addSessionToLayout(
  layout: CheckoutTerminalLayout,
  session: Session,
  split?: { targetSessionId: string; direction: SplitDirection; splitId: string },
): CheckoutTerminalLayout {
  const tabs = layout.tabs.map((tab) => {
    if (!split || !containsSession(tab.root, split.targetSessionId)) return tab;
    return {
      ...tab,
      root: replaceSession(tab.root, split.targetSessionId, {
        kind: "split",
        id: split.splitId,
        direction: split.direction,
        ratio: 0.5,
        first: { kind: "session", sessionId: split.targetSessionId },
        second: { kind: "session", sessionId: session.id },
      }),
    };
  });
  if (!split || !tabs.some((tab) => containsSession(tab.root, split.targetSessionId))) {
    tabs.push({ id: `layout:${session.id}`, root: { kind: "session", sessionId: session.id } });
  }
  const targetIndex = split ? layout.sessionOrder.indexOf(split.targetSessionId) + 1 : -1;
  const sessionOrder = [...layout.sessionOrder];
  sessionOrder.splice(targetIndex < 0 ? sessionOrder.length : targetIndex, 0, session.id);
  return { tabs, sessionOrder, activeTabId: split ? layout.activeTabId : `layout:${session.id}` };
}

export function removeSessionFromLayout(layout: CheckoutTerminalLayout, sessionId: string): CheckoutTerminalLayout {
  const tabs = layout.tabs.flatMap((tab) => {
    const root = removeSession(tab.root, sessionId);
    return root ? [{ ...tab, root }] : [];
  });
  const activeTabId = tabs.some((tab) => tab.id === layout.activeTabId) ? layout.activeTabId : (tabs[0]?.id ?? null);
  return {
    tabs,
    activeTabId,
    sessionOrder: layout.sessionOrder.filter((id) => id !== sessionId),
  };
}

export function reorderSession(
  layout: CheckoutTerminalLayout,
  sessionId: string,
  targetSessionId: string,
): CheckoutTerminalLayout {
  if (sessionId === targetSessionId) return layout;
  const sessionOrder = layout.sessionOrder.filter((id) => id !== sessionId);
  const targetIndex = sessionOrder.indexOf(targetSessionId);
  if (targetIndex < 0) return layout;
  sessionOrder.splice(targetIndex, 0, sessionId);
  return { ...layout, sessionOrder };
}

export function resizeSplit(layout: CheckoutTerminalLayout, splitId: string, ratio: number): CheckoutTerminalLayout {
  const nextRatio = Math.max(0.15, Math.min(0.85, ratio));
  return {
    ...layout,
    tabs: layout.tabs.map((tab) => ({ ...tab, root: resizeNode(tab.root, splitId, nextRatio) })),
  };
}

export function findSessionTab(layout: CheckoutTerminalLayout, sessionId: string): TerminalLayoutTab | undefined {
  return layout.tabs.find((tab) => containsSession(tab.root, sessionId));
}

export function firstSessionId(node: TerminalLayoutNode): string {
  return node.kind === "session" ? node.sessionId : firstSessionId(node.first);
}

function containsSession(node: TerminalLayoutNode, sessionId: string): boolean {
  return node.kind === "session"
    ? node.sessionId === sessionId
    : containsSession(node.first, sessionId) || containsSession(node.second, sessionId);
}

function replaceSession(
  node: TerminalLayoutNode,
  sessionId: string,
  replacement: TerminalLayoutNode,
): TerminalLayoutNode {
  if (node.kind === "session") return node.sessionId === sessionId ? replacement : node;
  return {
    ...node,
    first: replaceSession(node.first, sessionId, replacement),
    second: replaceSession(node.second, sessionId, replacement),
  };
}

function removeSession(node: TerminalLayoutNode, sessionId: string): TerminalLayoutNode | null {
  if (node.kind === "session") return node.sessionId === sessionId ? null : node;
  const first = removeSession(node.first, sessionId);
  const second = removeSession(node.second, sessionId);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

function resizeNode(node: TerminalLayoutNode, splitId: string, ratio: number): TerminalLayoutNode {
  if (node.kind === "session") return node;
  return {
    ...node,
    ratio: node.id === splitId ? ratio : node.ratio,
    first: resizeNode(node.first, splitId, ratio),
    second: resizeNode(node.second, splitId, ratio),
  };
}
