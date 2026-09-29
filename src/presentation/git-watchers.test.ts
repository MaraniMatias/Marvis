// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { defineComponent, h, ref } from "vue";
import type { Ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  watchGitRepo: vi.fn(),
  unwatchGitRepo: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  watchGitRepo: mocks.watchGitRepo,
  unwatchGitRepo: mocks.unwatchGitRepo,
}));

import { useGitWatchers } from "./git-watchers";

function checkout(id: string, isMissing = false): Checkout {
  return {
    id,
    repoId: "repo:a",
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing,
    sessions: [],
  };
}

function repo(id: string, checkouts: Checkout[], kind: Repo["kind"] = "git"): Repo {
  return {
    id,
    kind,
    name: id,
    root: `/${id}`,
    defaultBranch: "trunk",
    checkouts,
    createdAt: "now",
    lastOpenedAt: "now",
  };
}

function host(repos: Ref<Repo[]>) {
  return defineComponent({
    setup() {
      useGitWatchers(repos);
      return () => h("div");
    },
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

/** Lets the watcher promises and the Vue watcher callback both run. */
async function settle() {
  for (let round = 0; round < 5; round += 1) await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("useGitWatchers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.watchGitRepo.mockResolvedValue(undefined);
    mocks.unwatchGitRepo.mockResolvedValue(undefined);
  });

  it("watches every git repository once, not once per checkout and not once per edit", async () => {
    const repos = ref([repo("repo:a", [checkout("one"), checkout("two")]), repo("repo:b", [checkout("three")])]);
    const wrapper = mount(host(repos));
    await settle();

    // The sidebar names every checkout at once, so one watcher per repository covers all of
    // them, and a workspace edit that adds a terminal asks for nothing new.
    expect(mocks.watchGitRepo.mock.calls.map(([id]) => id)).toEqual(["repo:a", "repo:b"]);

    repos.value = [repos.value[0], repo("repo:b", [checkout("three"), checkout("four")])];
    await settle();

    expect(mocks.watchGitRepo.mock.calls.map(([id]) => id)).toEqual(["repo:a", "repo:b", "repo:b"]);
    wrapper.unmount();
  });

  it("never watches a plain folder or a repository whose every worktree is gone", async () => {
    const repos = ref([repo("folder", [checkout("one")], "plain")]);
    const wrapper = mount(host(repos));
    await settle();
    expect(mocks.watchGitRepo).not.toHaveBeenCalled();

    repos.value = [repo("gone", [checkout("one", true)])];
    await settle();
    expect(mocks.watchGitRepo).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("releases a repository when its last worktree is archived and picks it up again", async () => {
    const repos = ref([repo("repo:a", [checkout("one")])]);
    const wrapper = mount(host(repos));
    await settle();
    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(1);

    repos.value = [repo("repo:a", [])];
    await settle();
    expect(mocks.unwatchGitRepo).toHaveBeenCalledWith("repo:a");

    repos.value = [repo("repo:a", [checkout("one")])];
    await settle();
    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("starts one watcher per repository however often the workspace changes under it", async () => {
    const pending = deferred<void>();
    mocks.watchGitRepo.mockImplementation(() => pending.promise);
    const repos = ref([repo("repo:a", [checkout("one")])]);
    const wrapper = mount(host(repos));
    await settle();

    // Several edits arrive while the first watch is still in flight. The backend keeps the
    // first watcher it is given, so starting several would leave a worktree unwatched.
    repos.value = [repo("repo:a", [checkout("one"), checkout("two")])];
    await settle();
    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(1);

    pending.resolve(undefined);
    await settle();
    repos.value = [repo("repo:a", [checkout("one"), checkout("two"), checkout("three")])];
    await settle();
    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("asks again after a repository fails to start, so a later change can succeed", async () => {
    mocks.watchGitRepo.mockRejectedValueOnce(new Error("could not start repository watcher"));
    const repos = ref([repo("repo:a", [checkout("one")])]);
    const wrapper = mount(host(repos));
    await settle();
    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(1);

    repos.value = [repo("repo:a", [checkout("one"), checkout("two")])];
    await settle();

    expect(mocks.watchGitRepo).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
