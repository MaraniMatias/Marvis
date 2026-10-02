// @vitest-environment happy-dom
// The diff view is stubbed inline, the way the other component tests stub what they cannot mount.
/* eslint-disable vue/one-component-per-file, vue/require-default-prop */
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, provide, reactive, ref } from "vue";
import type { VNodeChild } from "vue";
import type { AgentSession } from "../domain/agent";
import { buildReviewMarkdown } from "../domain/review";
import type { ReviewNote } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { REVIEW_SENDER } from "../presentation/review-notes";
import type { NewReviewNoteInput, ReviewSender } from "../presentation/review-notes";
import { theme } from "../presentation/theme";

/** The library's `DiffFile`, as the stub above stands in for it. */
interface DiffFileStub {
  palette: string | undefined;
  notified: number;
}

const mocks = vi.hoisted(() => ({
  getGitDiff: vi.fn(),
  getGitDiffPage: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
}));

// The diff view renders through the library; the send, the header and the notes are what is
// under test here, so only the slots the library would fill are reproduced.
vi.mock("@git-diff-view/vue", async () => {
  const { defineComponent: component, h: createElement, ref: createRef } = await import("vue");
  return {
    DiffFile: class {
      /** What the library is told to paint in, which is the whole of what a palette switch reaches. */
      palette: string | undefined;
      notified = 0;
      initTheme(theme?: string) {
        this.palette = theme;
      }
      notifyAll() {
        this.notified += 1;
      }
      init() {}
      buildUnifiedDiffLines() {}
    },
    DiffModeEnum: { Unified: 4 },
    DiffViewWithMultiSelect: component({
      name: "DiffView",
      props: { extendData: { type: Object, default: () => ({}) }, diffFile: { type: Object, default: null } },
      setup: (
        props: { extendData: { newFile?: Record<string, { data: ReviewNote[] }> } },
        { slots }: { slots: Record<string, ((payload: never) => VNodeChild) | undefined> },
      ) => {
        const selectedRange = createRef<[number, number] | null>(null);
        return () =>
          createElement("div", { "data-testid": "diff-view" }, [
            slots.extend?.({ data: props.extendData?.newFile?.["1"]?.data ?? [] } as never),
            selectedRange.value
              ? slots.widget?.({
                  lineNumber: selectedRange.value[1],
                  fromLineNumber: selectedRange.value[0],
                  side: 2,
                  onClose: () => (selectedRange.value = null),
                } as never)
              : null,
            createElement(
              "button",
              { "data-testid": "select-diff-range", onClick: () => (selectedRange.value = [1, 3]) },
              "Select lines 1-3",
            ),
          ]);
      },
    }),
  };
});

vi.mock("reka-ui", async () => {
  const { h: createElement, defineComponent, inject, provide } = await import("vue");
  // The destination menu always renders here, so a target can be picked without driving the open
  // state, the way DocumentPane.test.ts stubs the toolbar's own popover.
  const passThrough = (name: string) => ({
    name,
    setup:
      (
        _props: unknown,
        context: { attrs: Record<string, unknown>; slots: Record<string, (() => unknown) | undefined> },
      ) =>
      () =>
        createElement("div", context.attrs, context.slots.default?.() as never),
  });
  // The select is stubbed with its wiring intact: the root holds the value and the rows report
  // a pick back to it, so a destination can be chosen here the way the keyboard would choose it.
  const select = Symbol("select");
  const selectRoot = defineComponent({
    name: "SelectRoot",
    props: { modelValue: String },
    emits: ["update:modelValue"],
    setup(props, { emit, slots }) {
      provide(select, (value: string) => emit("update:modelValue", value));
      return () => createElement("div", slots.default?.());
    },
  });
  const selectItem = defineComponent({
    name: "SelectItem",
    inheritAttrs: false,
    props: { value: String },
    // The row is an option here the way it is in the library, so a query for the destination
    // finds it by the role a screen reader would hear.
    setup(props, { attrs, slots }) {
      const pick = inject<((value: string) => void) | undefined>(select);
      return () =>
        createElement(
          "button",
          { role: "option", ...attrs, onClick: () => pick?.(props.value ?? "") },
          slots.default?.(),
        );
    },
  });
  return {
    PopoverRoot: passThrough("PopoverRoot"),
    PopoverTrigger: passThrough("PopoverTrigger"),
    PopoverPortal: passThrough("PopoverPortal"),
    PopoverContent: passThrough("PopoverContent"),
    SelectRoot: selectRoot,
    SelectTrigger: passThrough("SelectTrigger"),
    SelectValue: passThrough("SelectValue"),
    SelectPortal: passThrough("SelectPortal"),
    SelectContent: passThrough("SelectContent"),
    SelectViewport: passThrough("SelectViewport"),
    SelectItem: selectItem,
    SelectItemText: passThrough("SelectItemText"),
    SelectItemIndicator: passThrough("SelectItemIndicator"),
  };
});

import FileDiff from "./FileDiff.vue";

/** The session rows, named by the listbox that holds them: the destination select offers rows too. */
const SESSION_OPTION = '[aria-label="Send review to"] [role="option"]';

const checkout: Checkout = {
  id: "checkout:one",
  repoId: "repo:one",
  path: "/one",
  canonicalPath: "/one",
  isPrimary: true,
  changedFiles: 0,
  isMissing: false,
  sessions: [],
};

function snapshot(files: string[] = ["src/app.ts"]): ActiveGitSnapshot {
  return reactive<ActiveGitSnapshot>({
    checkoutId: checkout.id,
    status: {
      branch: "bug/1310-timeline",
      defaultBranch: "main",
      aheadCount: 1,
      files: files.map((path) => ({ path, status: "M" as const })),
    },
    loading: false,
    statusState: "ready",
    statusError: "",
    changesStatusError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
  });
}

function note(overrides: Partial<ReviewNote> = {}): ReviewNote {
  return {
    id: "note:1",
    checkoutId: checkout.id,
    path: "src/app.ts",
    side: "new",
    lineStart: 1,
    lineEnd: null,
    content: "revisit this calculation",
    code: "new",
    codeHash: "0000000000000001",
    outdated: false,
    roundId: null,
    status: "draft",
    createdAt: "1",
    updatedAt: "1",
    ...overrides,
  };
}

function session(id: string, overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id,
    checkoutId: checkout.id,
    title: id,
    busy: false,
    idleAt: 1,
    blockedOnPermission: false,
    agent: null,
    model: null,
    parentId: null,
    outcome: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function reviewApi(notes: ReviewNote[] = []) {
  return reactive({
    notes,
    addNote: vi.fn<(input: NewReviewNoteInput) => Promise<boolean>>(async () => true),
    updateNote: vi.fn(async () => true),
    deleteNote: vi.fn(async () => true),
    verifyAnchors: vi.fn(async () => true),
    clearOutdated: vi.fn(async () => true),
    resolveNote: vi.fn(async () => true),
  });
}

function senderStub(
  sessions: AgentSession[] = [session("ses_one")],
  targetId: string | null = "ses_one",
  unfinishedRounds = 0,
  reviewTarget: "markdown" | "opencode" = "opencode",
) {
  const sessionList = ref(sessions);
  const target = ref(targetId);
  const unfinished = ref(unfinishedRounds);
  const destination = ref(reviewTarget);
  const send = vi.fn(async () => undefined);
  const selectTarget = vi.fn();
  const selectReviewTarget = vi.fn((next: "markdown" | "opencode") => (destination.value = next));
  const sender: ReviewSender = {
    get target() {
      return destination.value;
    },
    get sessions() {
      return sessionList.value;
    },
    get targetId() {
      return target.value;
    },
    get unfinishedRounds() {
      return unfinished.value;
    },
    selectTarget,
    selectReviewTarget,
    send,
  };
  return { sender, send, selectTarget, selectReviewTarget };
}

/** The diff is reached through the shell, so the sender is what the shell provides. */
function mountDiff(props: Record<string, unknown>, sender: ReviewSender | null = senderStub().sender) {
  const harness = defineComponent({
    setup: () => {
      if (sender) provide(REVIEW_SENDER, sender);
      return () =>
        h(FileDiff, {
          checkout,
          gitSnapshot: snapshot(),
          review: reviewApi(),
          path: "src/app.ts",
          scrollTop: 0,
          ...props,
        });
    },
  });
  return mount(harness);
}

describe("FileDiff", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "diff --git a/src/app.ts b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({ path: "", startLine: 0, totalLines: 0, lines: [] });
  });

  it("names the file, or the change set and its branch", async () => {
    const single = mountDiff({});
    await flushPromises();
    expect(single.get("header").text()).toContain("src/app.ts");
    expect(single.get("header").text()).not.toContain("bug/1310-timeline");
    single.unmount();

    const all = mountDiff({ path: null });
    await flushPromises();
    expect(all.get("header").text()).toContain("All changes");
    expect(all.get("header").text()).toContain("bug/1310-timeline");
    all.unmount();
  });

  it("paints what is open in the theme in effect, and repaints it when the theme changes", async () => {
    const wrapper = mountDiff({});
    await flushPromises();
    const hunk = () => wrapper.getComponent({ name: "DiffView" }).props("diffFile") as DiffFileStub;
    expect(hunk().palette).toBe("dark");

    // A diff already on screen is told, rather than rebuilt: the library reads the theme off the
    // file it was handed, so a switch that only reached a new diff would leave this one painted in
    // the palette it was opened in.
    theme.value = "light";
    await flushPromises();
    expect(hunk().palette).toBe("light");
    expect(hunk().notified).toBe(1);
    wrapper.unmount();
    theme.value = "dark";
  });

  it("does not redraw a diff that git reports unchanged", async () => {
    const gitSnapshot = snapshot();
    const wrapper = mountDiff({ gitSnapshot });
    await flushPromises();

    const diffFile = () => wrapper.getComponent({ name: "DiffView" }).props("diffFile");
    const first = diffFile();
    expect(first).not.toBeNull();

    // Git reports every write in the workdir, not only in the file on screen, so a refresh lands
    // here after any save anywhere. It has to hand the view the same file: a new one takes the
    // open note composer down with it, which closed the composer the moment typing began. The
    // refresh is debounced, so the wait is for the timer rather than for the microtask queue.
    gitSnapshot.statusRevision += 1;
    await vi.waitFor(() => expect(mocks.getGitDiff).toHaveBeenCalledTimes(2));
    expect(diffFile()).toBe(first);

    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "diff --git a/src/app.ts b/src/app.ts\n@@ -1 +1 @@\n-old\n+other\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }],
    });
    gitSnapshot.statusRevision += 1;
    await vi.waitFor(() => expect(diffFile()).not.toBe(first));
    // A diff that did move is redrawn, or the view would keep showing lines git no longer has.
    expect(diffFile()).not.toBe(first);
    wrapper.unmount();
  });

  it("hides the header of a diff nested in the change-set stack, so the send is only one", async () => {
    const wrapper = mountDiff({ path: null });
    await flushPromises();
    await wrapper.get(".diff-file-header").trigger("click");
    await vi.waitFor(() => expect(wrapper.findAll('[data-testid="diff-view"]')).toHaveLength(1));
    // The nested diff is a diff of this same component, and the row above it already names the file.
    expect(wrapper.findAll("header")).toHaveLength(1);
    expect(wrapper.findAll('[data-testid="send-review"]')).toHaveLength(1);
    wrapper.unmount();
  });

  it("sends the notes that still need to reach the agent, and counts them", async () => {
    const review = reviewApi([
      note({ id: "note:1", status: "draft" }),
      note({ id: "note:2", status: "sent" }),
      // A resolved note is settled, and an outdated one is held back until asked for.
      note({ id: "note:3", status: "resolved" }),
      note({ id: "note:4", status: "draft", outdated: true }),
    ]);
    const stub = senderStub();
    const wrapper = mountDiff({ review }, stub.sender);
    await flushPromises();

    expect(wrapper.get('[data-testid="send-count"]').text()).toBe("2 notes · 1 draft");
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenCalledWith(["note:1", "note:2"], false);
    wrapper.unmount();
  });

  it("cannot send what the agent has nothing left to hear about", async () => {
    const stub = senderStub();
    const wrapper = mountDiff({ review: reviewApi([note({ id: "note:1", status: "resolved" })]) }, stub.sender);
    await flushPromises();

    expect(wrapper.get('[data-testid="send-review"]').attributes("disabled")).toBeDefined();
    expect(wrapper.find('[data-testid="send-count"]').exists()).toBe(false);
    expect(stub.send).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("holds an outdated note back until the checkbox asks for it", async () => {
    const review = reviewApi([note({ id: "note:1", status: "draft", outdated: true })]);
    const stub = senderStub();
    const wrapper = mountDiff({ review }, stub.sender);
    await flushPromises();

    expect(wrapper.get('[data-testid="send-review"]').attributes("disabled")).toBeDefined();
    await wrapper.get('[data-testid="include-outdated"]').setValue(true);
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenCalledWith(["note:1"], false);
    wrapper.unmount();
  });

  it("offers a destination only above one session, and reports rounds left unconfirmed", async () => {
    const one = senderStub([session("ses_one")]);
    const single = mountDiff({ review: reviewApi([note()]) }, one.sender);
    await flushPromises();
    expect(single.find('[data-testid="send-target"]').exists()).toBe(false);
    single.unmount();

    const longTitle = "Review this session with a descriptive title longer than one line";
    const many = senderStub([session("ses_one"), session("ses_two", { title: longTitle })], "ses_two", 2);
    const wrapper = mountDiff({ review: reviewApi([note()]) }, many.sender);
    await flushPromises();

    expect(wrapper.get('[data-testid="send-review"]').classes()).toContain("marvis-button-md");
    expect(wrapper.get('[data-testid="send-target"]').classes()).toContain("marvis-select");
    expect(wrapper.find(".session-target-menu").exists()).toBe(true);
    expect(wrapper.get('[data-testid="send-target"]').attributes("title")).toBe(longTitle);
    const options = wrapper.findAll(SESSION_OPTION);
    expect(options.map((option) => option.text())).toEqual(["ses_one", longTitle]);
    expect(options[1].attributes("title")).toBe(longTitle);
    expect(options[1].get(".session-target-label").text()).toBe(longTitle);
    expect(options[1].classes()).toContain("session-target-option");
    // The chosen one is the one that carries the check, so the list says where it already points.
    expect(options[1].attributes("aria-selected")).toBe("true");
    expect(options[1].find("svg").exists()).toBe(true);
    expect(options[0].attributes("aria-selected")).toBe("false");
    expect(wrapper.get('[data-testid="unfinished-rounds"]').text()).toBe("2 rounds not finished");

    await options[0].trigger("click");
    expect(many.selectTarget).toHaveBeenCalledWith("ses_one");
    wrapper.unmount();
  });

  it("exports the notes as markdown without a session, a busy dialog, or a round count", async () => {
    const busy = session("ses_one", { title: "review one", busy: true, idleAt: null });
    const stub = senderStub([busy], "ses_one", 3, "markdown");
    const wrapper = mountDiff({ review: reviewApi([note()]) }, stub.sender);
    await flushPromises();

    expect(wrapper.find('[data-testid="send-target"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="send-busy"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="unfinished-rounds"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="send-review"]').text()).toBe("Export as Markdown");
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenCalledWith(["note:1"], false);
    wrapper.unmount();
  });

  it("saves a selected diff range and keeps its range in Markdown export", async () => {
    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "@@ -1,3 +1,3 @@\n first\n-second\n+second changed\n third\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 5,
      hunks: [{ startLine: 0, endLine: 5, title: "@@ -1,3 +1,3 @@" }],
    });
    const review = reviewApi();
    const wrapper = mountDiff({ review });
    await flushPromises();

    await wrapper.get('[data-testid="select-diff-range"]').trigger("click");
    expect(wrapper.get('form[aria-label="New review note"]').text()).toContain("new lines 1-3");
    await wrapper.get('textarea[aria-label="Review note"]').setValue("keep these lines together");
    await wrapper.get('form[aria-label="New review note"]').trigger("submit");
    await flushPromises();

    expect(review.addNote).toHaveBeenCalledWith({
      path: "src/app.ts",
      side: "new",
      lineStart: 1,
      lineEnd: 3,
      content: "keep these lines together",
      code: "first\nsecond changed\nthird",
    });
    const saved = review.addNote.mock.calls[0][0];
    const markdown = buildReviewMarkdown(
      [
        note({
          lineStart: saved.lineStart,
          lineEnd: saved.lineEnd ?? null,
          code: saved.code,
          content: saved.content,
        }),
      ],
      { date: "2026-03-14" },
    );
    expect(markdown).toContain("```ts{1-3}\nfirst\nsecond changed\nthird\n```");
    wrapper.unmount();
  });

  it("lets the checkout choose a review destination", async () => {
    const stub = senderStub([session("ses_one")], "ses_one", 0, "markdown");
    const wrapper = mountDiff({ review: reviewApi([note()]) }, stub.sender);
    await flushPromises();

    // The rows render without the list being opened, and they are found through the listbox's
    // own name because the session menu above offers options of its own.
    await wrapper
      .findAll('[aria-label="Review destination"] [role="option"]')
      .find((row) => row.text() === "OpenCode")!
      .trigger("click");
    expect(stub.selectReviewTarget).toHaveBeenCalledWith("opencode");
    expect(wrapper.get('[data-testid="send-review"]').text()).toBe("Send to opencode");
    wrapper.unmount();
  });

  it("the markdown target does not change which notes are included", async () => {
    const review = reviewApi([
      note({ id: "draft" }),
      note({ id: "sent", status: "sent" }),
      note({ id: "resolved", status: "resolved" }),
      note({ id: "outdated", outdated: true }),
    ]);
    const stub = senderStub([], null, 0, "markdown");
    const wrapper = mountDiff({ review }, stub.sender);
    await flushPromises();

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenLastCalledWith(["draft", "sent"], false);
    await wrapper.get('[data-testid="include-outdated"]').setValue(true);
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenLastCalledWith(["draft", "sent", "outdated"], false);
    wrapper.unmount();
  });

  it("lists the newest session first, and only searches a list long enough to need it", async () => {
    const two = senderStub([
      session("ses_old", { title: "old work", updatedAt: 1 }),
      session("ses_new", { title: "new work", updatedAt: 5 }),
    ]);
    const short = mountDiff({ review: reviewApi([note()]) }, two.sender);
    await flushPromises();
    // A send goes to the session that was last used, so the newest one leads the list.
    expect(short.findAll(SESSION_OPTION).map((option) => option.text())).toEqual(["new work", "old work"]);
    expect(short.find('input[aria-label="Search sessions"]').exists()).toBe(false);
    short.unmount();

    const sessions = Array.from({ length: 6 }, (_, index) =>
      session(`ses_${index}`, { title: index === 3 ? "CanvasForm work" : `session ${index}`, updatedAt: index }),
    );
    const six = senderStub(sessions, "ses_0");
    const wrapper = mountDiff({ review: reviewApi([note()]) }, six.sender);
    await flushPromises();

    expect(wrapper.findAll(SESSION_OPTION).map((option) => option.text())).toEqual([
      "session 5",
      "session 4",
      "CanvasForm work",
      "session 2",
      "session 1",
      "session 0",
    ]);
    const search = wrapper.get('input[aria-label="Search sessions"]');
    await search.setValue("canvas");
    expect(wrapper.findAll(SESSION_OPTION).map((option) => option.text())).toEqual(["CanvasForm work"]);
    await search.setValue("nothing");
    expect(wrapper.findAll(SESSION_OPTION)).toHaveLength(0);
    expect(wrapper.get(".menu-note").text()).toBe('No session matches "nothing".');
    wrapper.unmount();
  });

  it("asks what to do when the target is mid-task, and can be talked out of it", async () => {
    const busy = session("ses_one", { title: "review one", busy: true, idleAt: null });
    const stub = senderStub([busy]);
    const wrapper = mountDiff({ review: reviewApi([note()]) }, stub.sender);
    await flushPromises();

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    // The agent cannot be interrupted, so the click opens the choice instead of sending.
    expect(stub.send).not.toHaveBeenCalled();
    const choice = wrapper.get('[data-testid="send-busy"]');
    expect(choice.text()).toContain("“review one” is mid-task. Sending now lands inside its current turn");
    expect(choice.text()).toContain("This OpenCode version cannot cancel a turn.");

    await wrapper.get('[data-testid="send-not-now"]').trigger("click");
    expect(wrapper.find('[data-testid="send-busy"]').exists()).toBe(false);
    expect(stub.send).not.toHaveBeenCalled();

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await wrapper.get('[data-testid="send-queue"]').trigger("click");
    expect(stub.send).toHaveBeenCalledWith(["note:1"], true);
    expect(wrapper.find('[data-testid="send-busy"]').exists()).toBe(false);

    // Landing inside the turn is the other answer, and it is the one the default offers.
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await wrapper.get('[data-testid="send-now"]').trigger("click");
    expect(stub.send).toHaveBeenLastCalledWith(["note:1"], false);
    wrapper.unmount();
  });

  it("sends straight away when the target is free", async () => {
    const stub = senderStub([session("ses_one"), session("ses_two")]);
    const wrapper = mountDiff({ review: reviewApi([note()]) }, stub.sender);
    await flushPromises();

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    expect(stub.send).toHaveBeenCalledWith(["note:1"], false);
    expect(wrapper.find('[data-testid="send-busy"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("draws nothing to send when no shell is above the diff", async () => {
    const wrapper = mount(FileDiff, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review: reviewApi([note()]),
        path: "src/app.ts",
        scrollTop: 0,
      },
    });
    await flushPromises();
    expect(wrapper.find('[data-testid="send-review"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("only offers to resolve a note the diff can back, and only after it was delivered", async () => {
    // Hunk mode: the whole patch is in memory, so an absent anchor really is a deleted line.
    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "@@ -1,2 +1,2 @@\n const a = 1;\n+const b = 2;\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1,2 +1,2 @@" }],
    });
    const review = reviewApi([
      note({ id: "note:sent", status: "sent" }),
      note({ id: "note:draft", status: "draft" }),
      note({ id: "note:drifted", status: "sent", outdated: true }),
    ]);
    const wrapper = mountDiff({ review });
    await vi.waitFor(() => expect(wrapper.text()).toContain("revisit this calculation"));

    // Without a verdict the button is not offered: marking something resolved has to mean
    // something was checked, and a draft was never delivered in the first place.
    expect(wrapper.findAll('[aria-label^="Mark note on line"]')).toHaveLength(1);
    expect(wrapper.get('[aria-label="Mark note on line 1 as resolved"]')).toBeTruthy();
    expect(wrapper.get('[data-testid="diff-view"]').text()).toContain("The line this note points at is unchanged.");
    wrapper.unmount();
  });
});
