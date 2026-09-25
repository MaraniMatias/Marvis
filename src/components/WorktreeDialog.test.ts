// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Repo, WorkspaceState } from "../domain/workspace";
import WorktreeDialog from "./WorktreeDialog.vue";

const ipc = vi.hoisted(() => ({
  createWorktree: vi.fn(),
  getWorktreeDefaults: vi.fn(),
  getWorktreeRemovalInfo: vi.fn(),
  removeWorktree: vi.fn(),
  setDefaultBranch: vi.fn(),
}));

vi.mock("../lib/ipc", () => ipc);

const checkout: Checkout = {
  id: "checkout:primary",
  repoId: "repo:test",
  path: "/test",
  canonicalPath: "/test",
  isPrimary: true,
  branch: "trunk",
  changedFiles: 0,
  isMissing: false,
  sessions: [],
};

const repo: Repo = {
  id: "repo:test",
  kind: "git",
  name: "test",
  root: "/test",
  defaultBranch: "trunk",
  checkouts: [checkout],
  createdAt: "now",
  lastOpenedAt: "now",
};

const workspace: WorkspaceState = { repos: [repo], activeCheckoutId: checkout.id, activeSessionId: null };

describe("WorktreeDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("submits the editable name, branch, and external location, then opens a shell", async () => {
    ipc.getWorktreeDefaults.mockResolvedValue({ location: "/Users/test/.marvis/worktrees", defaultBranch: "trunk" });
    ipc.createWorktree.mockResolvedValue({ workspace, checkoutId: "checkout:new" });
    const wrapper = mount(WorktreeDialog, {
      props: { open: true, mode: "create", repo, checkout },
    });
    await flushPromises();

    const maliciousName = "../../$(touch should-not-run); *";
    await wrapper.get("input[autofocus]").setValue(maliciousName);
    await wrapper.findAll("input")[1].setValue("feature/safe");
    await wrapper.findAll("input")[2].setValue("/Users/test/worktrees");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(ipc.createWorktree).toHaveBeenCalledWith(
      checkout.id,
      maliciousName,
      "feature/safe",
      "/Users/test/worktrees",
    );
    expect(wrapper.emitted("workspaceUpdated")).toEqual([[workspace]]);
    expect(wrapper.emitted("requestShell")).toEqual([["checkout:new"]]);
  });

  it("shows dirty files and active sessions, defaults to keeping the branch, and requires confirmation", async () => {
    ipc.getWorktreeRemovalInfo.mockResolvedValue({
      checkoutId: "checkout:feature",
      isPrimary: false,
      isMissing: false,
      branch: "feature",
      dirtyFiles: ["uncommitted.txt"],
      unmergedCommits: 2,
      activeSessions: [{ id: "session:1", type: "shell", name: "zsh · feature" }],
      activeAgentSessions: [],
    });
    ipc.removeWorktree.mockResolvedValue({ workspace });
    const worktree = { ...checkout, id: "checkout:feature", isPrimary: false, branch: "feature" };
    const wrapper = mount(WorktreeDialog, {
      props: { open: true, mode: "remove", repo, checkout: worktree },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("uncommitted.txt");
    expect(wrapper.text()).toContain("zsh · feature");
    expect((wrapper.get("select").element as HTMLSelectElement).value).toBe("keep");
    const removeButton = wrapper.findAll("button").find((button) => button.text() === "Remove worktree");
    expect(removeButton?.attributes("disabled")).toBeDefined();

    await wrapper.findAll('input[type="checkbox"]')[0].setValue(true);
    await wrapper.findAll('input[type="checkbox"]')[1].setValue(true);
    await wrapper.get("select").setValue("delete");
    await wrapper
      .findAll("button")
      .find((button) => button.text() === "Remove worktree")
      ?.trigger("click");
    await flushPromises();

    expect(ipc.removeWorktree).toHaveBeenCalledWith(
      "checkout:feature",
      true,
      ["uncommitted.txt"],
      ["session:1"],
      "feature",
      2,
      true,
    );
    expect(wrapper.emitted("workspaceUpdated")).toEqual([[workspace]]);
  });

  it("offers a shell for commit or stash and blocks removal while an agent is active", async () => {
    ipc.getWorktreeRemovalInfo.mockResolvedValue({
      checkoutId: "checkout:feature",
      isPrimary: false,
      isMissing: false,
      branch: "feature",
      dirtyFiles: ["draft.txt"],
      unmergedCommits: 0,
      activeSessions: [],
      activeAgentSessions: [{ id: "agent:1", type: "agent", name: "Agent 1" }],
    });
    const worktree = { ...checkout, id: "checkout:feature", isPrimary: false, branch: "feature" };
    const wrapper = mount(WorktreeDialog, {
      props: { open: true, mode: "remove", repo, checkout: worktree },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("Agent 1");
    expect(
      wrapper
        .findAll("button")
        .find((button) => button.text() === "Remove worktree")
        ?.attributes("disabled"),
    ).toBeDefined();
    await wrapper.get("button.underline").trigger("click");

    expect(wrapper.emitted("requestShell")).toEqual([["checkout:feature"]]);
    expect(ipc.removeWorktree).not.toHaveBeenCalled();
  });
});
