// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { reactive } from "vue";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import ChangesPane from "./ChangesPane.vue";

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
      props: { checkout, gitSnapshot: snapshot(), selectedPath: null },
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
      props: { checkout, gitSnapshot, selectedPath: null },
    });
    expect(wrapper.text()).toContain("Loading Git status");
    gitSnapshot.statusState = "error";
    gitSnapshot.changesStatusError = "Could not compare branches";
    await flushPromises();
    expect(wrapper.text()).toContain("Could not compare branches");
    wrapper.unmount();
  });
});
