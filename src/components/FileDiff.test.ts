// @vitest-environment happy-dom
// The diff view is stubbed inline, the way the other component tests stub what they cannot mount.
/* eslint-disable vue/one-component-per-file */
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, provide, reactive, ref } from "vue";
import type { VNodeChild } from "vue";
import type { AgentSession } from "../domain/agent";
import type { ReviewNote } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { REVIEW_SENDER } from "../presentation/review-notes";
import type { ReviewSender } from "../presentation/review-notes";

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
  const { defineComponent: component, h: createElement } = await import("vue");
  return {
    DiffFile: class {
      initTheme() {}
      init() {}
      buildUnifiedDiffLines() {}
    },
    DiffModeEnum: { Unified: 4 },
    DiffView: component({
      name: "DiffView",
      props: { extendData: { type: Object, default: () => ({}) } },
      setup:
        (
          props: { extendData: { newFile?: Record<string, { data: ReviewNote[] }> } },
          { slots }: { slots: Record<string, ((payload: never) => VNodeChild) | undefined> },
        ) =>
        () =>
          createElement("div", { "data-testid": "diff-view" }, [
            slots.extend?.({ data: props.extendData?.newFile?.["1"]?.data ?? [] } as never),
          ]),
    }),
  };
});

import FileDiff from "./FileDiff.vue";

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
    viewedError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: vi.fn(async () => undefined),
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
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function reviewApi(notes: ReviewNote[] = []) {
  return reactive({
    notes,
    addNote: vi.fn(async () => true),
    updateNote: vi.fn(async () => true),
    deleteNote: vi.fn(async () => true),
    verifyAnchors: vi.fn(async () => true),
    clearOutdated: vi.fn(async () => true),
    resolveNote: vi.fn(async () => true),
  });
}

function senderStub(sessions: AgentSession[] = [session("ses_one")], targetId = "ses_one", unfinishedRounds = 0) {
  const sessionList = ref(sessions);
  const target = ref(targetId);
  const unfinished = ref(unfinishedRounds);
  const send = vi.fn(async () => undefined);
  const selectTarget = vi.fn();
  const sender: ReviewSender = {
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
    send,
  };
  return { sender, send, selectTarget };
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
          active: true,
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
    expect(single.find('select[aria-label="Send review to"]').exists()).toBe(false);
    single.unmount();

    const many = senderStub([session("ses_one"), session("ses_two", { title: "review two" })], "ses_two", 2);
    const wrapper = mountDiff({ review: reviewApi([note()]) }, many.sender);
    await flushPromises();

    const picker = wrapper.get('select[aria-label="Send review to"]');
    expect(picker.findAll("option").map((option) => option.text())).toEqual(["ses_one", "review two"]);
    expect(wrapper.get('[data-testid="unfinished-rounds"]').text()).toBe("2 rounds not finished");
    await picker.setValue("ses_one");
    expect(many.selectTarget).toHaveBeenCalledWith("ses_one");
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
        active: true,
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
