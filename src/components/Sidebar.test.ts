// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Repo } from "../domain/workspace";
import Sidebar from "./Sidebar.vue";

describe("Sidebar actions and checkout states", () => {
  it("offers Open directory and marks missing checkouts", async () => {
    const repo: Repo = {
      id: "repo:test",
      kind: "git",
      name: "test",
      root: "/test",
      checkouts: [
        {
          id: "checkout:gone",
          repoId: "repo:test",
          path: "/test/gone",
          canonicalPath: "/test/gone",
          isPrimary: false,
          changedFiles: 0,
          isMissing: true,
          sessions: [],
        },
      ],
      createdAt: "now",
      lastOpenedAt: "now",
    };
    const wrapper = mount(Sidebar, {
      props: { repos: [repo], activeCheckoutId: null, activeSessionId: null, isOpening: false },
    });

    expect(wrapper.get('button[aria-label="Open directory"]')).toBeDefined();
    expect(wrapper.text()).toContain("Open directory");
    expect(wrapper.text()).toContain("Missing");
    await wrapper.get('button[aria-label="Open directory"]').trigger("click");
    expect(wrapper.emitted("openFolder")).toHaveLength(1);
  });
});
