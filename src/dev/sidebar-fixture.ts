/**
 * A dev-only harness that draws the real `Sidebar` with invented props, for looking at.
 *
 * It exists because the panel's layout decisions are about pixels — where a name starts, whether a
 * glyph sits on the title's line, whether anything is cut — and none of them can be checked by a
 * test that lays nothing out. Open `/sidebar-fixture.html` on the dev server to see it.
 *
 * It is inert by construction:
 *
 * - It never reaches the backend. The Tauri internals are stubbed below before anything imports
 *   them, so the panel's own data calls answer from this file and the workspace on disk, the
 *   preferences and every live terminal are untouched.
 * - Every event is a no-op, so clicking, dragging, right clicking and typing a rename change
 *   nothing outside this page.
 * - Nothing here is in the production bundle: the page is a separate HTML entry that only the dev
 *   server serves, and `main.ts` never imports this file.
 */
import { createApp, defineComponent, h, onMounted, ref, watch } from "vue";
import type { Checkout, Repo, Session, TerminalSessionStatus } from "../domain/workspace";
import Sidebar from "../components/Sidebar.vue";
import "../marvis.css";
import "../style.css";
import type { TerminalAgentRow, TerminalAgentSession } from "../presentation/agent-sessions";

/**
 * The one thing the panel would otherwise ask the backend for, answered from here.
 *
 * The panel asks for Git's counts for every checkout and subscribes to a change signal; the counts
 * are the only answer it draws from, so the harness supplies its own and leaves the real Git
 * alone. The subscription is answered with a plain number because that is what it returns, and the
 * panel unlistens through the same stub.
 */
const FIXTURE_DIFF_TOTALS: Record<string, { additions: number; deletions: number }> = {
  "checkout:states": { additions: 244, deletions: 110 },
  // Four-figure and two-figure counts side by side, so the monospaced columns can be read down the
  // list as well as on one row, and a zero where Git has nothing to say.
  "checkout:figs-feature": { additions: 4855, deletions: 1102 },
  "checkout:figs-other": { additions: 63, deletions: 26 },
  "checkout:figs-root": { additions: 1809, deletions: 276 },
  "checkout:figs-gone": { additions: 0, deletions: 0 },
  "checkout:shells": { additions: 12, deletions: 3 },
  "checkout:fold-open": { additions: 244, deletions: 110 },
  "checkout:fold-closed": { additions: 63, deletions: 26 },
};

/** Tauri publishes this on the window; it is untyped here because nothing else reads it. */
const windowWithInternals = window as unknown as Record<string, unknown>;
windowWithInternals.__TAURI_INTERNALS__ = {
  invoke: async (command: string) => {
    if (command === "git_checkout_diff_stats") return FIXTURE_DIFF_TOTALS;
    if (command === "git_diff_stats") return [];
    if (command === "plugin:event|listen") return 1;
    if (command === "plugin:event|unlisten") return null;
    throw new Error(`the sidebar fixture answers no command: ${command}`);
  },
  transformCallback: (callback: unknown) => callback,
  convertFileSrc: (path: string) => path,
  unregisterCallback: () => undefined,
};

function repoOf(id: string, name: string, kind: Repo["kind"], root: string, checkouts: Checkout[]): Repo {
  return { id, kind, name, root, checkouts, createdAt: "now", lastOpenedAt: "now" };
}

function shellOf(id: string, checkoutId: string, name = "zsh"): Session {
  return { id, type: "shell", checkoutId, name, createdAt: "now", status: "active" };
}

/** A checkout with terminals in it, named after its branch the way a worktree is. */
function checkoutOf(input: {
  id: string;
  repoId: string;
  path: string;
  branch?: string;
  primary?: boolean;
  sessions?: Session[];
}): Checkout {
  return {
    id: input.id,
    repoId: input.repoId,
    path: input.path,
    canonicalPath: input.path,
    isPrimary: input.primary ?? false,
    branch: input.branch,
    changedFiles: 0,
    isMissing: false,
    sessions: input.sessions ?? [],
  };
}

/** A session title too long for a terminal title, so the terminal carries it cut. */
const LONG_TITLE = "Plan de implementación para la sidebar y sus tres estados, con un nombre que no cabe en la fila";
/** What OpenCode's TUI writes for it: the first 37 characters and an ellipsis, past 40. */
const LONG_TITLE_AS_TERMINAL_TITLE = `OC | ${LONG_TITLE.slice(0, 37)}…`;
const LONG_BRANCH = "bug/13133933180-fix-login-redirect-loop-on-token-refresh";

/**
 * The panel's own clock, read once so every elapsed time in a frame is the same age and a
 * screenshot taken twice of the same URL is the same picture.
 */
const NOW = Date.now();

/** An agent session as the service would report it, `secondsAgo` old. */
function agentSessionOf(input: {
  title: string;
  label: string | null;
  attention: "none" | "busy" | "blocked" | "failed";
  secondsAgo: number;
}): TerminalAgentSession {
  return {
    title: input.title,
    agent: input.label ? { label: input.label, color: null, attention: input.attention } : null,
    running: input.attention === "busy",
    updatedAt: NOW - input.secondsAgo * 1000,
  };
}

interface Case {
  /** What this case is for, drawn above it. */
  label: string;
  repos: Repo[];
  activeCheckoutId: string | null;
  activeSessionId: string | null;
  sessionRuntimeStatuses: Record<string, TerminalSessionStatus>;
  agentRows: Record<string, TerminalAgentRow>;
  /**
   * The branches this frame draws folded away, by the title the group row shows.
   *
   * The fold is component state on purpose — a fold is one person's view of a list they are reading
   * right now, not a preference they set once — so there is no prop to hand it through. The harness
   * presses the same chevron a person would press, once, after the frame is on the page.
   */
  folded?: string[];
}

/**
 * Every row the panel can draw, one case at a time.
 *
 * The terminal titles are here on purpose: this is the only place a fixture can know what a
 * terminal has open, and every agent row below is honest because it was given that. A terminal
 * whose title named no session is drawn as unidentified rather than idle, which is what a terminal
 * Marvis did not spawn is.
 */
const CASES: Case[] = [
  {
    label: "All five states: working, waiting, failed, idle agent, unidentified agent",
    repos: [
      repoOf("repo:states", "states", "git", "/Users/dev/states", [
        checkoutOf({
          id: "checkout:states",
          repoId: "repo:states",
          path: "/Users/dev/states",
          branch: "main",
          primary: true,
          sessions: [
            shellOf("session:working", "checkout:states"),
            shellOf("session:waiting", "checkout:states"),
            shellOf("session:failed", "checkout:states"),
            shellOf("session:idle-agent", "checkout:states"),
            shellOf("session:unidentified", "checkout:states"),
            shellOf("session:running", "checkout:states"),
            shellOf("session:idle-shell", "checkout:states"),
          ],
        }),
      ]),
    ],
    // The selected row is the working one: selection and a state on the same row at once is the
    // case where a selection that was a second, louder accent would show.
    activeCheckoutId: "checkout:states",
    activeSessionId: "session:working",
    sessionRuntimeStatuses: {
      "session:working": {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: "OC | Copy ids into the lists that already have them",
      },
      "session:waiting": {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: "OC | Review the duplicated rows in the sidebar",
      },
      "session:failed": {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: "OC | Fix the truncation of the workdir name",
      },
      "session:idle-agent": {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: "OC | Short one",
      },
      // OpenCode with no session title: a home screen, or a terminal Marvis did not spawn.
      "session:unidentified": { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
      "session:running": { state: "running", foregroundProcess: true, foregroundApp: "pnpm" },
      "session:idle-shell": { state: "running", foregroundProcess: false, terminalTitle: "dev@mbp:~/Trabajo" },
    },
    agentRows: {
      "checkout:states": {
        sessions: [
          // A mode longer than its share of the row, to see the detail give way before the name.
          agentSessionOf({
            title: "Copy ids into the lists that already have them",
            label: "general-purpose-orchestrator",
            attention: "busy",
            secondsAgo: 12,
          }),
          agentSessionOf({
            title: "Review the duplicated rows in the sidebar",
            label: "plan",
            attention: "blocked",
            secondsAgo: 8 * 60,
          }),
          agentSessionOf({
            title: "Fix the truncation of the workdir name",
            label: "code-reviewer-with-a-very-long-name",
            attention: "failed",
            secondsAgo: 62 * 60,
          }),
          agentSessionOf({ title: "Short one", label: "coder", attention: "none", secondsAgo: 4 * 60 * 60 }),
          // A session nobody's terminal has open, which is what a stale worktree looks like: it
          // must not appear on any row above.
          agentSessionOf({
            title: "An older session from before the rewrite",
            label: null,
            attention: "none",
            secondsAgo: 60,
          }),
        ],
      },
    },
  },
  {
    label: "Two identical idle shells, a renamed one and a running process in one worktree",
    repos: [
      repoOf("repo:shells", "shells", "git", "/Users/dev/shells", [
        checkoutOf({
          id: "checkout:shells",
          repoId: "repo:shells",
          path: "/Users/dev/shells/.worktrees/feat/13133933180-duplicate-rows",
          branch: "feat/13133933180-duplicate-rows",
          sessions: [
            shellOf("session:first", "checkout:shells"),
            shellOf("session:second", "checkout:shells"),
            { ...shellOf("session:renamed", "checkout:shells"), name: "deploy" },
            shellOf("session:serving", "checkout:shells"),
          ],
        }),
      ]),
    ],
    activeCheckoutId: "checkout:shells",
    activeSessionId: null,
    sessionRuntimeStatuses: {
      "session:first": { state: "running", foregroundProcess: false },
      "session:second": { state: "running", foregroundProcess: false },
      "session:renamed": { state: "running", foregroundProcess: false },
      "session:serving": { state: "running", foregroundProcess: true, foregroundApp: "pnpm" },
    },
    agentRows: {},
  },
  {
    label: "A folded group beside an open one, and a worktree with no terminals at all",
    repos: [
      repoOf("repo:fold", "fold", "git", "/Users/dev/fold", [
        checkoutOf({
          id: "checkout:fold-open",
          repoId: "repo:fold",
          path: "/Users/dev/fold",
          branch: "main",
          primary: true,
          sessions: [
            shellOf("session:f-open", "checkout:fold-open"),
            shellOf("session:f-open-shell", "checkout:fold-open"),
          ],
        }),
        checkoutOf({
          id: "checkout:fold-closed",
          repoId: "repo:fold",
          path: "/Users/dev/fold/.worktrees/feat/13133933180-collapse-me",
          branch: "feat/13133933180-collapse-me",
          sessions: [
            shellOf("session:f-closed", "checkout:fold-closed"),
            shellOf("session:f-closed-shell", "checkout:fold-closed"),
          ],
        }),
        // A worktree nothing is open in: its group is a row and a chevron and nothing else.
        checkoutOf({
          id: "checkout:fold-empty",
          repoId: "repo:fold",
          path: "/Users/dev/fold/.worktrees/wip",
          branch: "wip",
        }),
      ]),
    ],
    activeCheckoutId: "checkout:fold-open",
    activeSessionId: null,
    folded: ["feat/13133933180-collapse-me"],
    sessionRuntimeStatuses: {
      "session:f-open": { state: "running", foregroundProcess: true, foregroundApp: "pnpm" },
      "session:f-open-shell": { state: "running", foregroundProcess: false },
      "session:f-closed": { state: "running", foregroundProcess: false },
      "session:f-closed-shell": { state: "running", foregroundProcess: false },
    },
    agentRows: {},
  },
  {
    label: "A long branch with counts beside a short one, a missing directory and a plain folder",
    repos: [
      repoOf("repo:figs", "figs", "git", "/Users/dev/figs", [
        checkoutOf({
          id: "checkout:figs-feature",
          repoId: "repo:figs",
          path: "/Users/dev/figs/.worktrees/feat/13133933180-copy-id-into-the-lists",
          branch: LONG_BRANCH,
          sessions: [shellOf("session:f-child", "checkout:figs-feature")],
        }),
        checkoutOf({
          id: "checkout:figs-other",
          repoId: "repo:figs",
          path: "/Users/dev/figs/.worktrees/bug/13133933180-disable-adguard-in-every-frame",
          branch: "bug/13133933180-disable-adguard-in-every-frame",
          sessions: [],
        }),
        checkoutOf({
          id: "checkout:figs-root",
          repoId: "repo:figs",
          path: "/Users/dev/figs",
          branch: "main",
          primary: true,
          sessions: [shellOf("session:f-root", "checkout:figs-root")],
        }),
        // The directory is gone: the row keeps its reason for existing and its one way out.
        { ...checkoutOf({ id: "checkout:figs-gone", repoId: "repo:figs", path: "/Users/dev/gone" }), isMissing: true },
      ]),
    ],
    activeCheckoutId: "checkout:figs-feature",
    activeSessionId: null,
    // Four-figure and two-figure counts side by side, so the monospaced columns can be read down
    // the list as well as on one row.
    sessionRuntimeStatuses: {
      "session:f-child": { state: "running", foregroundProcess: true, foregroundApp: "pnpm" },
      "session:f-root": { state: "running", foregroundProcess: false },
    },
    agentRows: {},
  },
  {
    label: "A long session title the terminal carries cut, beside a plain folder with no Git",
    repos: [
      repoOf("repo:notes", "notes", "plain", "/Users/dev/Documents/notes", [
        checkoutOf({
          id: "checkout:notes",
          repoId: "repo:notes",
          path: "/Users/dev/Documents/notes",
          sessions: [shellOf("session:notes", "checkout:notes"), shellOf("session:notes-shell", "checkout:notes")],
        }),
      ]),
    ],
    activeCheckoutId: "checkout:notes",
    activeSessionId: null,
    sessionRuntimeStatuses: {
      "session:notes": {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "opencode",
        terminalTitle: LONG_TITLE_AS_TERMINAL_TITLE,
      },
      "session:notes-shell": { state: "running", foregroundProcess: false },
    },
    agentRows: {
      "checkout:notes": {
        sessions: [agentSessionOf({ title: LONG_TITLE, label: "plan", attention: "busy", secondsAgo: 45 })],
      },
    },
  },
];

/** The panel's own minimum width, which is the narrowest a reader can ever make it. */
const MIN_WIDTH = 240;

/** The width the sidebar is actually set to, which is what a reviewer usually wants first. */
const DEFAULT_WIDTH = 297;

/**
 * What to draw, read from the query string so a screenshot tool can ask for one thing at a time.
 *
 * `/sidebar-fixture.html?theme=light&case=states&width=240` is a single frame; no parameters draws
 * everything. Only this page's own selection is read, and a parameter that names nothing draws
 * nothing rather than falling back, so a typo in a screenshot request is visible instead of quietly
 * producing the whole matrix again.
 */
interface Filters {
  theme: "dark" | "light";
  cases: number[];
  widths: number[];
}

/** The short names a query can use, in the order the cases are declared. */
const CASE_KEYS = ["states", "shells", "fold", "figs", "notes"] as const;

function readFilters(): Filters {
  const query = new URLSearchParams(window.location.search);
  const wanted = query.get("case");
  const cases = wanted
    ? CASE_KEYS.map((key, index) => (wanted.split(",").includes(key) ? index : -1)).filter((index) => index >= 0)
    : CASE_KEYS.map((_, index) => index);
  const widths = (query.get("width")?.split(",") ?? []).map(Number).filter((width) => Number.isFinite(width));
  return {
    theme: query.get("theme") === "light" ? "light" : "dark",
    cases,
    widths: widths.length ? widths : [MIN_WIDTH, DEFAULT_WIDTH],
  };
}

/**
 * A theme control, because the palette is written on the root element and there is no other way to
 * see the light one: `:root[data-theme="light"]` is the whole of the switch. The attribute is set on
 * this page's document and nothing else, which is the same as what the app's own preference does.
 *
 * It is a control rather than a fixed value so the page is usable by hand, and it follows the query
 * when there is one so a screenshot of either theme is the same URL shape as every other frame.
 */
const filters = readFilters();
const theme = ref<"dark" | "light">(filters.theme);
watch(
  theme,
  (value) => {
    document.documentElement.dataset.theme = value;
  },
  { immediate: true },
);

const Fixture = defineComponent({
  components: { Sidebar },
  setup() {
    /** Each frame on the page with the case it drew, so a fold is pressed in its own frame only. */
    const frames: { root: HTMLElement; folded: string[] }[] = [];

    onMounted(() => {
      for (const frame of frames) {
        for (const branch of frame.folded) {
          frame.root.querySelector<HTMLElement>(`button.workdir-fold[aria-label="Collapse ${branch}"]`)?.click();
        }
      }
    });

    return () =>
      h("div", { class: "fixture" }, [
        h("header", { class: "fixture-head" }, [
          h(
            "p",
            { style: { margin: "0 0 8px" } },
            "Dev fixture: the real sidebar with invented props. Nothing here reaches the backend, and every button in the panel does nothing.",
          ),
          h("label", { style: { display: "inline-flex", gap: "6px", alignItems: "center" } }, [
            "Theme",
            h(
              "select",
              {
                value: theme.value,
                onChange: (event: Event) => {
                  theme.value = (event.target as HTMLSelectElement).value === "light" ? "light" : "dark";
                },
              },
              (["dark", "light"] as const).map((option) => h("option", { value: option }, option)),
            ),
          ]),
          h(
            "p",
            { style: { margin: "8px 0 0", opacity: "0.7" } },
            `One frame per case × width: ${CASE_KEYS.join(", ")} × ${filters.widths.join(", ")}px. Narrow it with ?theme=light&case=states&width=240`,
          ),
        ]),
        filters.cases.flatMap((index) =>
          filters.widths.map((width) => {
            const entry = CASES[index];
            if (!entry) return null;
            // The section is bounded to the frame's own width, so a long case label wraps inside
            // it instead of stretching the grid column and pushing every other frame off the
            // screen. The label is about this frame, not about the page.
            return h(
              "section",
              { class: "fixture-case", key: `${entry.label}-${width}`, style: { width: `${width}px` } },
              [
                h("h2", { class: "fixture-case-label" }, `${entry.label} — ${width}px`),
                h(
                  "div",
                  {
                    class: "fixture-frame",
                    ref: (element: unknown) =>
                      frames.push({ root: element as HTMLElement, folded: entry.folded ?? [] }),
                  },
                  [
                    h(Sidebar, {
                      repos: entry.repos,
                      activeCheckoutId: entry.activeCheckoutId,
                      activeSessionId: entry.activeSessionId,
                      isOpening: false,
                      sessionRuntimeStatuses: entry.sessionRuntimeStatuses,
                      agentRows: entry.agentRows,
                      // Nothing this page draws is wired to anything.
                      onOpenFolder: () => undefined,
                      onSelectCheckout: () => undefined,
                      onSelectSession: () => undefined,
                      onCreateWorktree: () => undefined,
                      onNewTerminal: () => undefined,
                      onRemoveWorktree: () => undefined,
                      onCloseWorkdir: () => undefined,
                      onCloseMissing: () => undefined,
                      onRestoreArchived: () => undefined,
                      onCloseSession: () => undefined,
                      onRenameSession: () => undefined,
                      onMoveSession: () => undefined,
                    }),
                  ],
                ),
              ],
            );
          }),
        ),
      ]);
  },
});

createApp(Fixture).mount("#app");
