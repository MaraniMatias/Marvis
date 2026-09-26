// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
import type { AgentSession } from "../domain/agent";
import type { ReviewNote, ReviewRound } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import ChangesPane from "./ChangesPane.vue";

function reviewNote(overrides: Partial<ReviewNote> = {}): ReviewNote {
  return {
    id: "note:1",
    checkoutId: checkout.id,
    path: "src/new.ts",
    side: "new",
    lineStart: 10,
    lineEnd: null,
    content: "revisit this calculation",
    code: "const result = a + b;",
    codeHash: "0000000000000001",
    outdated: false,
    roundId: null,
    status: "draft",
    createdAt: "1",
    updatedAt: "1",
    ...overrides,
  };
}

function reviewApi(notes: ReviewNote[], rounds: ReviewRound[] = []) {
  return reactive({
    notes,
    rounds,
    markSent: vi.fn(async () => true),
  });
}

function agentSessionFixture(id: string, overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id,
    checkoutId: "checkout:repo",
    title: `agent ${id}`,
    busy: false,
    idleAt: 1,
    blockedOnPermission: false,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

const checkout: Checkout = {
  id: "checkout:repo",
  repoId: "repo:repo",
  path: "/repo",
  canonicalPath: "/repo",
  isPrimary: true,
  changedFiles: 0,
  isMissing: false,
  sessions: [],
};

function snapshot(): ActiveGitSnapshot {
  return reactive<ActiveGitSnapshot>({
    checkoutId: checkout.id,
    status: {
      branch: "feature",
      defaultBranch: "trunk",
      aheadCount: 1,
      files: [
        { path: "src/new.ts", oldPath: "src/old.ts", status: "R" },
        { path: "deleted.txt", status: "D" },
      ],
    },
    viewedPaths: ["src/new.ts"],
    loading: false,
    statusState: "ready",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: async () => undefined,
  });
}

describe("ChangesPane", () => {
  it("shows branch comparison context, filters renamed files, and opens the current path", async () => {
    const wrapper = mount(ChangesPane, {
      props: { checkout, gitSnapshot: snapshot(), selectedPath: null, scrollTop: 0 },
    });

    expect(wrapper.text()).toContain("vs trunk · 2 changed");
    expect(wrapper.text()).toContain("1/2 viewed");
    expect(wrapper.text()).toContain("src/old.ts → src/new.ts");
    expect(wrapper.text()).toContain("deleted.txt");

    await wrapper.get('input[aria-label="Filter changed files"]').setValue("old");
    await flushPromises();
    expect(wrapper.text()).toContain("src/new.ts");
    expect(wrapper.text()).not.toContain("deleted.txt");
    await wrapper.get('[aria-label="Changed files"] button').trigger("click");
    expect(wrapper.emitted("openChange")).toEqual([[{ checkoutId: checkout.id, path: "src/new.ts" }]]);
    wrapper.unmount();
  });

  it("shows loading and error states from the shared status snapshot", async () => {
    const gitSnapshot = snapshot();
    gitSnapshot.statusState = "loading";
    const wrapper = mount(ChangesPane, {
      props: { checkout, gitSnapshot, selectedPath: null, scrollTop: 0 },
    });
    expect(wrapper.text()).toContain("Loading Git status");
    gitSnapshot.statusState = "error";
    gitSnapshot.changesStatusError = "Could not compare branches";
    await flushPromises();
    expect(wrapper.text()).toContain("Could not compare branches");
    wrapper.unmount();
  });

  it("restores and reports the changed-file list position", async () => {
    const wrapper = mount(ChangesPane, {
      props: { checkout, gitSnapshot: snapshot(), selectedPath: null, scrollTop: 84 },
    });
    await flushPromises();
    const viewport = wrapper.get('[aria-label="Changed files"]');
    expect((viewport.element as HTMLElement).scrollTop).toBe(84);
    (viewport.element as HTMLElement).scrollTop = 112;
    await viewport.trigger("scroll");
    expect(wrapper.emitted("scrollPositionChanged")).toEqual([[112]]);
    wrapper.unmount();
  });

  it("offers the review round only when there are notes and ships the whole checkout", async () => {
    const withoutNotes = mount(ChangesPane, {
      props: { checkout, gitSnapshot: snapshot(), review: reviewApi([]), selectedPath: null, scrollTop: 0 },
    });
    expect(withoutNotes.find('[data-testid="send-review"]').exists()).toBe(false);
    withoutNotes.unmount();

    const review = reviewApi([
      reviewNote({ id: "note:1" }),
      reviewNote({ id: "note:2", status: "sent", lineStart: 20 }),
    ]);
    const wrapper = mount(ChangesPane, {
      props: { checkout, gitSnapshot: snapshot(), review, selectedPath: null, scrollTop: 0 },
    });
    expect(wrapper.get('[data-testid="review-note-count"]').text()).toContain("2 notes · 1 draft");

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    expect(review.markSent).toHaveBeenCalledWith(["note:1", "note:2"]);
    expect(wrapper.emitted("sendReview")).toEqual([[["note:1", "note:2"], false]]);
    wrapper.unmount();
  });

  it("asks the user what to do instead of interrupting an agent that is mid-task", async () => {
    const review = reviewApi([reviewNote()]);
    const wrapper = mount(ChangesPane, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review,
        agentSessions: [agentSessionFixture("ses_busy", { busy: true })],
        agentTargetId: "ses_busy",
        selectedPath: null,
        scrollTop: 0,
      },
    });

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    // Held back: nothing was marked sent, so "not now" cannot leave notes that look
    // delivered but were never sent.
    expect(wrapper.find('[data-testid="busy-agent-choice"]').exists()).toBe(true);
    expect(review.markSent).not.toHaveBeenCalled();
    expect(wrapper.emitted("sendReview")).toBeUndefined();

    await wrapper.get('[data-testid="send-not-now"]').trigger("click");
    await flushPromises();
    expect(wrapper.find('[data-testid="busy-agent-choice"]').exists()).toBe(false);
    expect(wrapper.emitted("sendReview")).toBeUndefined();

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="send-queued"]').trigger("click");
    await flushPromises();
    expect(review.markSent).toHaveBeenCalledWith(["note:1"]);
    expect(wrapper.emitted("sendReview")).toEqual([[["note:1"], true]]);
    expect(wrapper.find('[data-testid="busy-agent-choice"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("lets the user interrupt anyway, and only then", async () => {
    const review = reviewApi([reviewNote()]);
    const wrapper = mount(ChangesPane, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review,
        agentSessions: [agentSessionFixture("ses_busy", { busy: true })],
        agentTargetId: "ses_busy",
        selectedPath: null,
        scrollTop: 0,
      },
    });

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="send-now"]').trigger("click");
    await flushPromises();
    expect(wrapper.emitted("sendReview")).toEqual([[["note:1"], false]]);
    wrapper.unmount();

    // An agent that is not mid-task is not asked about at all.
    const idle = mount(ChangesPane, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review: reviewApi([reviewNote()]),
        agentSessions: [agentSessionFixture("ses_idle")],
        agentTargetId: "ses_idle",
        selectedPath: null,
        scrollTop: 0,
      },
    });
    await idle.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    expect(idle.find('[data-testid="busy-agent-choice"]').exists()).toBe(false);
    expect(idle.emitted("sendReview")).toEqual([[["note:1"], false]]);
    idle.unmount();
  });

  it("offers a target only when there is a real choice, and names a blocked agent", async () => {
    const single = agentSessionFixture("ses_one", { busy: true });
    const one = mount(ChangesPane, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review: reviewApi([reviewNote()]),
        agentSessions: [single],
        agentTargetId: "ses_one",
        selectedPath: null,
        scrollTop: 0,
      },
    });
    // One live agent is not a choice, so no selector is shown.
    expect(one.find('[data-testid="review-agent-target"]').exists()).toBe(false);
    one.unmount();

    const blocked = agentSessionFixture("ses_blocked", { blockedOnPermission: true });
    const busy = agentSessionFixture("ses_busy", { busy: true });
    const many = mount(ChangesPane, {
      props: {
        checkout,
        gitSnapshot: snapshot(),
        review: reviewApi(
          [reviewNote()],
          [
            {
              id: "round:1",
              checkoutId: checkout.id,
              sessionId: "ses_blocked",
              status: "dispatched",
              marker: "marvis-review:round:1",
              noteIds: ["note:1"],
              createdAt: "1",
              updatedAt: "1",
            },
          ],
        ),
        agentSessions: [blocked, busy],
        agentTargetId: "ses_blocked",
        selectedPath: null,
        scrollTop: 0,
      },
    });
    const selector = many.get('[data-testid="review-agent-target"]');
    expect(selector.text()).toContain("needs permission");
    expect(selector.text()).toContain("(working)");
    // A permission nobody can answer must be visible, not silently pending.
    expect(many.text()).toContain("cannot answer");
    expect(many.get('[data-testid="pending-rounds"]').text()).toContain("1 round not finished");

    await selector.setValue("ses_busy");
    expect(many.emitted("selectAgentTarget")).toEqual([["ses_busy"]]);
    many.unmount();
  });

  it("holds outdated notes out of the round until the user opts back in", async () => {
    const review = reviewApi([
      reviewNote({ id: "note:1" }),
      reviewNote({ id: "note:2", lineStart: 20, outdated: true }),
    ]);
    const wrapper = mount(ChangesPane, {
      props: { checkout, gitSnapshot: snapshot(), review, selectedPath: null, scrollTop: 0 },
    });
    expect(wrapper.text()).toContain("1 outdated note left out");

    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    expect(review.markSent).toHaveBeenCalledWith(["note:1"]);
    expect(wrapper.emitted("sendReview")).toEqual([[["note:1"], false]]);

    await wrapper.get('[data-testid="include-outdated"]').setValue(true);
    await wrapper.get('[data-testid="send-review"]').trigger("click");
    await flushPromises();
    expect(wrapper.emitted("sendReview")).toEqual([
      [["note:1"], false],
      [["note:1", "note:2"], false],
    ]);
    wrapper.unmount();
  });
});
